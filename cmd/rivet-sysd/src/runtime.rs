//! **Facts the machine publishes for its own programs to read** — `/run/rivet`.
//!
//! # Why this is not `crate::etc`, and the distinction is the point
//!
//! The first build of the display window put it in `/etc/display-hours` and the
//! maintainer asked the right question on 2026-09-02: why the display window
//! needed an `/etc` projection at all (.rulings/ R-266). It did not, and the
//! rule it broke is worth writing down because nothing had stated it as a rule
//! before:
//!
//! > **`/etc` on a Rivet exists because C runtimes demand exact paths.** Every
//! > file `crate::etc` renders is there under duress — musl's `localtime_r`
//! > reads `/etc/localtime` and nowhere else, its resolver reads
//! > `/etc/resolv.conf`, `getpwnam` reads `/etc/passwd`, `getusershell` reads
//! > `/etc/shells`. It is a **compatibility surface**, not a configuration
//! > surface, which is why `ADR-007` §3.6.2 calls a file there a view and never
//! > an interface and why §3.5 remounts the whole thing read-only.
//!
//! No C library looks for a display schedule. Its only reader is
//! `rivet_sdk::schedule`, which is Rivet's own code in somebody else's program
//! — so the file belongs in **Rivet's own runtime namespace**, beside the
//! journal socket, the kiosk socket, the query socket and `reached`.
//!
//! The test for whether something belongs in `crate::etc` is therefore: **name
//! the C runtime that will not look anywhere else.** If there is not one, it
//! belongs here.
//!
//! # Why a file at all, rather than an environment variable
//!
//! `ADR-036`'s first article is that an application is *handed* what it needs,
//! and `RIVET_DISPLAY` is handed as a variable — so a variable would have been
//! consistent. A file wins on one concrete point: **a variable is fixed at
//! `exec` and a file is not.** `ADR-039` §11 item 5 records that the window
//! currently takes effect on the next restart, and closing that gap means a
//! reader noticing a change without being restarted, which a variable forecloses
//! and a file does not.

use rivet_core::journal::{Severity, Value};


use crate::journal::Journal;
use crate::kernel::{Errno, Kernel};

/// This module's name in the journal.
const SELF_NAME: &str = "rivet-sysd";

/// **`0o644`, and it has to be**: the readers are unprivileged application
/// processes running as their own uid (`ADR-015` §5.1), and a window they
/// cannot read is a window they treat as absent — which leaves a building lit
/// all night with nothing in any log to say why.
const MODE: u32 = 0o644;

fn note(k: &mut dyn Kernel, journal: &mut Journal, sev: Severity, msg: &str, fields: Vec<(String, Value)>) {
    let corr = journal.next_correlation();
    journal.emit(k, sev, SELF_NAME, &corr, msg, fields);
}

/// What one publish attempt did, for the caller's journal line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Published {
    /// The document names a window and the file now holds it.
    Wrote,
    /// The document names no window, so there is no file.
    ///
    /// **Not an error and not a no-op.** A machine whose `[display]` table was
    /// removed must lose the file, or its screens keep going dark on a schedule
    /// nobody can find in the document any more.
    Withdrew,
    /// The write or the removal failed; the reason is in the journal.
    Failed,
}

/// **Publish what version of Rivet this machine is** —
/// `rivet_core::generated::OS_VERSION_PATH`.
///
/// `CARGO_PKG_VERSION` here is the IMAGE's, and that is the whole point: this
/// binary ships inside the image, so the two cannot disagree. A program that
/// used its own version instead would be right only while it happened to be
/// built from the same tree — see the const's own note for the launcher that
/// was not.
///
/// Written once at start rather than every pass: an image does not change
/// under a running machine. Failure is reported and survivable; a reader that
/// cannot find the file says so rather than guessing.
pub fn publish_os_version(k: &mut dyn Kernel, journal: &mut Journal) {
    let path = rivet_core::generated::OS_VERSION_PATH;
    let contents = format!("{}\n", env!("CARGO_PKG_VERSION"));
    if let Err(why) = k.write_file(path, contents.as_bytes(), MODE) {
        note(
            k,
            journal,
            Severity::Warn,
            "this machine's version could not be published, so anything that wanted to say \
             which Rivet it is running on will have to say it does not know",
            vec![
                ("path".to_string(), Value::Str(path.to_string())),
                ("error".to_string(), Value::Str(format!("{why:?}"))),
            ],
        );
    }
}

/// **What each installed application declares it understands** — the manifest
/// half of `ADR-038` §4.3's *complete on the machine* validation.
///
/// Read from the manifests on disk rather than carried on `ServiceDef`, and
/// that is a deliberate trade: a field there would have to be threaded through
/// **thirty-seven** construction sites, most of them image services that
/// declare no settings and never will. This is one read per installed
/// application, once per delivery, and delivery happens on a reload rather than
/// on every convergence pass.
///
/// **An application whose manifest cannot be read declares nothing**, which
/// means every setting for it is refused and journalled rather than delivered
/// blind. `crate::applications::read_one` has already reported the unreadable
/// manifest by then and has not started the program, so this adds no second
/// complaint about the same fact.
pub fn declared_settings(
    k: &mut dyn Kernel,
    managed: &[crate::supervise::Managed],
) -> Vec<(rivet_core::service::ServiceName, String, Vec<String>)> {
    let mut out = Vec::new();
    for def in managed.iter().map(|m| &m.def) {
        // Image services have no volume and no manifest on disk - they are
        // defined in `crate::definitions` - so they declare nothing and are
        // skipped rather than looked up and reported as missing.
        let Some(volume) = def.volume.as_deref() else {
            continue;
        };
        let Some(name) = rivet_core::service::ServiceName::new(&def.name) else {
            continue;
        };
        // **An image service with a home declares from the image's table**
        // (`ADR-060`): it has the home and the settings file an application
        // has, and no manifest to declare the keys in. Only while it is
        // enabled — `crate::applications::provide_image_homes` makes no home
        // for one the document leaves off, so there is nowhere to write.
        if def.origin == crate::service::Origin::Image {
            if def.enabled
                && let Some(home) = rivet_core::service::image_service_home(&def.name)
            {
                let declared = home.settings.iter().map(|k| (*k).to_string()).collect();
                out.push((name, volume.to_string(), declared));
            }
            continue;
        }
        let declared = k
            .read_file(&rivet_core::service::manifest_path(volume, &name))
            .ok()
            .and_then(|text| {
                rivet_core::package::application::Manifest::parse(text.as_bytes()).ok()
            })
            .map(|manifest| manifest.settings().to_vec())
            .unwrap_or_default();
        out.push((name, volume.to_string(), declared));
    }
    out
}

/// **Hand every installed application what the document says it is set to** —
/// `ADR-038` §6.
///
/// # Why the supervisor and not the launcher
///
/// The maintainer asked whether the launcher might pass settings in. It should
/// not: **the launcher's subject is the display lease** (`PLAN.md` §106), so
/// routing settings through it would leave every application that does not draw
/// — a fleet server, a controller, a network addon — with no path at all, and
/// most Rivet machines are headless (`ADR-036` §8.4).
///
/// The supervisor already hands over `RIVET_APP_DATA` and the listening
/// sockets. This is that mechanism a fourth time, which is what `ADR-004` §6
/// keeps calling *"the same mechanism again"*.
///
/// # This is where the check actually happens
///
/// `rivet-config` kept every well-formed key it was given, because on a
/// workstation it cannot know what is installed or what a manifest declares
/// (`ADR-038` §4.3). **Here both are to hand**, so this is the complete
/// validation: a key the application never declared is not delivered, and is
/// journalled by name with what to do about it.
///
/// # Nothing here is fatal
///
/// A directory that cannot be written, a document naming an application that is
/// not installed, a key nobody declared: all reported, none fatal. `ADR-038`
/// §4.4's containment reaches this far too — a setting must never be the reason
/// a machine stops converging.
pub fn deliver_settings(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    document: &rivet_config::Document,
    installed: &[(rivet_core::service::ServiceName, String, Vec<String>)],
) {
    // What the document could not even carry - reported here rather than in
    // `rivet-config`, because this is the first place with a journal.
    for ignored in document.applications().ignored() {
        note(
            k,
            journal,
            Severity::Warn,
            "a line under an application in the configuration document could not be carried, \
             and was IGNORED rather than refusing the document (ADR-038 §4.4)",
            vec![
                ("application".to_string(), Value::Str(ignored.application.clone())),
                ("key".to_string(), Value::Str(ignored.key.clone())),
                ("reason".to_string(), Value::Str(ignored.why.to_string())),
            ],
        );
    }

    for (name, volume, declared) in installed {
        let values: Vec<(String, String)> = document
            .applications()
            .get(name.as_str())
            .map(|settings| {
                settings
                    .values()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            })
            .unwrap_or_default();

        // **Written even when there is nothing to write.** An application whose
        // settings were removed must see them gone, and a file left behind from
        // a previous document would be a value nobody has asked for since.
        let (text, rejected) = rivet_core::settings::render(declared, &values);
        let path = rivet_core::settings::path(volume, name);
        // **~~This path is below `data/`, which the application owns and can
        // write.~~ Not since 2026-09-04** — `ADR-038` §8.1 moved it up beside
        // `current`, into the root-owned `0755` tree the installer makes, so
        // there is nowhere for an unprivileged program to plant a link and the
        // class `PLAN.md` §123.2 item 2 guarded is gone rather than guarded.
        //
        // **The guarded write is KEPT, and it is no longer what makes this
        // safe.** The root-owned parent is. It stays because it costs one flag,
        // because `an_applications_settings_are_written_with_the_symlink_guards`
        // is a tripwire on a file that has already moved once, and because a
        // future edit that moves it back would otherwise be silent.
        match k.write_file_nofollow(&path, text.as_bytes(), rivet_core::settings::MODE) {
            Ok(()) => {
                if !values.is_empty() || !rejected.is_empty() {
                    note(
                        k,
                        journal,
                        Severity::Info,
                        "an application was handed its settings",
                        vec![
                            ("application".to_string(), Value::Str(name.as_str().to_string())),
                            ("delivered".to_string(), Value::Int(
                                i64::try_from(values.len() - rejected.len()).unwrap_or(0),
                            )),
                        ],
                    );
                }
            }
            Err(why) => note(
                k,
                journal,
                Severity::Warn,
                "an application's settings could not be written, so it runs on whatever it \
                 was handed last - or on nothing. It is not stopped over this",
                vec![
                    ("application".to_string(), Value::Str(name.as_str().to_string())),
                    ("path".to_string(), Value::Str(path)),
                    ("error".to_string(), Value::Str(format!("{why:?}"))),
                ],
            ),
        }

        // **The check the declaration exists for**, and it is loud on purpose:
        // a misspelled key is otherwise accepted, stored, provisioned to a
        // fleet, and produces an application that behaves as though it were
        // never set, with nothing reporting it (`ADR-038` §3.1).
        for refusal in rejected {
            note(
                k,
                journal,
                Severity::Warn,
                "a setting in the configuration document was NOT delivered",
                vec![
                    ("application".to_string(), Value::Str(name.as_str().to_string())),
                    ("key".to_string(), Value::Str(refusal.key)),
                    ("reason".to_string(), Value::Str(refusal.why.to_string())),
                ],
            );
        }
    }

    // **A table for an application this machine does not have is not an
    // error**, and saying so once is worth more than silence: provisioning a
    // fleet before the software is installed is the case the region exists for
    // (`ADR-038` §4.1), so this is an ordinary state of a machine mid-rollout.
    for settings in document.applications().all() {
        if !installed.iter().any(|(name, _, _)| name.as_str() == settings.name()) {
            note(
                k,
                journal,
                Severity::Info,
                "the document configures an application this machine has not installed. Kept \
                 rather than dropped - provisioning ahead of an install is what the region is \
                 for - and it will be delivered when the application arrives",
                vec![(
                    "application".to_string(),
                    Value::Str(settings.name().to_string()),
                )],
            );
        }
    }
}

/// **Hand the name service its projection of the document** — `ADR-047` §6.
///
/// `rivet-dnsd` runs confined under its own uid and may not read the machine
/// document, so this writes it `rivet_config::Document::dns_projection` —
/// `version`, `hostname` and `[dns]`, rendered by the document's own
/// serialiser — at `rivet_core::name_service::path`, root-owned and `0644`,
/// beside its settings file.
///
/// **Wherever settings are delivered, and from the same list.** `installed` is
/// [`declared_settings`]' answer, which holds an image service with a home only
/// while it is enabled — so a machine that does not run the name service gets
/// no file, and one whose document turns it on at a reload gets one before it
/// starts. Rewritten on every delivery, so a changed table is what the service
/// reads at its next start.
///
/// `hostname` is the name the machine carries — the one `crate::naming`
/// settled and handed to the supervisor — and not the document's `hostname`
/// key, which is absent on a machine whose name is derived.
///
/// A failed write is reported and survived, for [`deliver_settings`]' reason;
/// the service then reads whatever was written last, or refuses for want of a
/// file.
pub fn deliver_name_service_document(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    document: &rivet_config::Document,
    hostname: Option<&str>,
    installed: &[(rivet_core::service::ServiceName, String, Vec<String>)],
) {
    let Some((name, volume, _)) = installed
        .iter()
        .find(|(name, _, _)| name.as_str() == rivet_core::service::DNSD_SERVICE_NAME)
    else {
        return;
    };
    let hostname = hostname.and_then(rivet_core::hostname::Hostname::new);
    let text = document.dns_projection(hostname.as_ref()).to_toml();
    let path = rivet_core::name_service::path(volume, name);
    if let Err(why) = k.write_file_nofollow(&path, text.as_bytes(), rivet_core::name_service::MODE) {
        note(
            k,
            journal,
            Severity::Warn,
            "the name service's projection of the document could not be written, so it serves \
             whatever table it was handed last - or refuses to start for want of one",
            vec![
                ("svc_name".to_string(), Value::Str(name.as_str().to_string())),
                ("path".to_string(), Value::Str(path)),
                ("error".to_string(), Value::Str(format!("{why:?}"))),
            ],
        );
    }
}

/// **Hand the mDNS responder its projection** — `ADR-017` §3.
///
/// `rivet-mdnsd` runs confined under its own uid and may read neither the
/// machine document nor the DHCP leases, so this writes it
/// [`crate::mdns::Inputs::projection`] — the machine's name and each
/// interface's address, exactly the reading the supervisor compares to decide
/// a restart — at `rivet_core::mdns::path`, root-owned and `0644`.
///
/// **Wherever the name service's projection is delivered, from the same
/// list**, so a machine that does not run the responder gets no file and one
/// whose document turns it on at a reload gets one before it starts. Every
/// reload rewrites it, and every lease `rivet-dhcp-hook` writes causes a
/// reload, so a first lease reaches a responder already waiting for it
/// without a restart.
///
/// `inputs` is `None` only before the supervisor has recorded any, and then
/// nothing is written. A failed write is reported and survived, for
/// [`deliver_settings`]' reason; the responder then reads whatever was written
/// last, or refuses for want of a file.
pub fn deliver_mdns_projection(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    inputs: Option<&crate::mdns::Inputs>,
    installed: &[(rivet_core::service::ServiceName, String, Vec<String>)],
) {
    let Some((name, volume, _)) = installed
        .iter()
        .find(|(name, _, _)| name.as_str() == rivet_core::service::MDNSD_SERVICE_NAME)
    else {
        return;
    };
    let Some(inputs) = inputs else {
        return;
    };
    let text = inputs.projection().render();
    let path = rivet_core::mdns::path(volume, name);
    if let Err(why) = k.write_file_nofollow(&path, text.as_bytes(), rivet_core::mdns::MODE) {
        note(
            k,
            journal,
            Severity::Warn,
            "the mDNS responder's projection could not be written, so it announces whatever it \
             was handed last - or refuses to start for want of it",
            vec![
                ("svc_name".to_string(), Value::Str(name.as_str().to_string())),
                ("path".to_string(), Value::Str(path)),
                ("error".to_string(), Value::Str(format!("{why:?}"))),
            ],
        );
    }
}

/// **The service `rivet_core::proxy`'s table is written for.**
///
/// A route's *reader* is the proxy, and the proxy reads it out of its own
/// tree — so there is one name here rather than a search, and a machine that
/// does not run the proxy gets no file written anywhere. An image service since
/// `ADR-060`, with the home it had as an application.
const PROXY_SERVICE: &str = rivet_core::service::PROXYD_SERVICE_NAME;

/// One installed application, as [`publish_routes`] needs it.
///
/// **A snapshot rather than a borrow of `sup.services()`**, for the same reason
/// [`declared_settings`] returns owned values: the journal is reached through
/// the same `&mut` the service list is, so a caller holding both would be
/// holding two borrows of one supervisor. [`installed_applications`] takes the
/// snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub name: rivet_core::service::ServiceName,
    pub volume: String,
}

/// Every managed service with a home on a volume, by name and volume — every
/// installed application, and every image service that runs under its own
/// identity (`ADR-060`).
///
/// **Any other image service has neither**, so it is skipped rather than looked
/// up and reported as missing — the same filter [`declared_settings`] applies.
/// One with a home is kept, because the proxy's is where the routing table
/// goes; it has no manifest, so [`publish_routes`] finds no route declared by
/// it, which is right — the proxy routes to others and is not behind itself.
///
/// An image service is kept only while it is enabled, for [`declared_settings`]'
/// reason: one the document leaves off has no home to write a table into.
#[must_use]
pub fn installed_applications(managed: &[crate::supervise::Managed]) -> Vec<Installed> {
    managed
        .iter()
        .map(|m| &m.def)
        .filter(|def| def.origin != crate::service::Origin::Image || def.enabled)
        .filter_map(|def| {
            Some(Installed {
                name: rivet_core::service::ServiceName::new(&def.name)?,
                volume: def.volume.as_deref()?.to_string(),
            })
        })
        .collect()
}

/// **Derive `rivet-proxyd`'s routing table and hand it over** — `ADR-046` §5,
/// `R-133`, `R-152`.
///
/// # Nothing here is configured, and that is the whole design
///
/// Every row comes from an installed application's **signed manifest** — its
/// `proxy-path`, its first TCP `listen` port, and its `proxy-auth`. An operator
/// cannot add a route, cannot move one, and cannot make one public: doing any
/// of those means shipping a new artifact that somebody signed.
///
/// It is `deliver_settings` a second time and deliberately so — a view of
/// desired state, rewritten every convergence, written root-owned beside
/// `current` rather than into the `0700` directory the application owns
/// (`ADR-038` §8.1, and `rivet_core::proxy::path` carries the reason).
///
/// # Nothing here is fatal
///
/// A machine with no proxy installed, a manifest that will not parse, an
/// application that asked for a path and declared no TCP socket, two
/// applications asking for one path: every one is reported and none stops a
/// convergence. `ADR-038` §4.4's containment, pointed at routing — a route must
/// never be the reason a machine stops converging, because the machine is
/// reached *through* it.
///
/// # The return value is whether the derived table carries a `plain:` rule
///
/// `false` on a machine with no proxy installed. [`reconcile_proxy_listen`] is
/// the caller, right after this function returns, and is what turns the
/// answer into `rivet-proxyd`'s stored `listen` — `ADR-046` §7, `ADR-061` §7,
/// `.rulings/` R-221.
pub fn publish_routes(k: &mut dyn Kernel, journal: &mut Journal, installed: &[Installed]) -> bool {
    // **Where the file goes, and the reason it is looked up rather than
    // composed.** The proxy's home is on the volume its definition names
    // (`ADR-060`), and the definition is what the supervisor holds.
    let Some(destination) = installed
        .iter()
        .find(|one| one.name.as_str() == PROXY_SERVICE)
        .map(|one| rivet_core::proxy::path(&one.volume, &one.name))
    else {
        // **A machine that does not run the proxy.** Writing a table for a
        // service the document leaves off would put a file in a tree nothing
        // owns, and there is no proxy for a plain rule to matter to.
        return false;
    };

    let mut wanted = Vec::new();
    for one in installed {
        let (volume, name) = (&one.volume, &one.name);
        let Some(manifest) = k
            .read_file(&rivet_core::service::manifest_path(volume, name))
            .ok()
            .and_then(|text| {
                rivet_core::package::application::Manifest::parse(text.as_bytes()).ok()
            })
        else {
            continue;
        };
        let Some(path) = manifest.proxy_path() else {
            continue;
        };
        // **The FIRST TCP socket, and an application with none is refused a
        // route rather than given one to a port it does not hold.** A
        // `proxy-path` beside `listen = ["unix:api"]` is a manifest asking for
        // something a path proxy cannot do, and silently omitting the row would
        // leave the author with a route that 404s and nothing saying why.
        //
        // `Manifest::route_port` is that rule, and the manifest readers ask it
        // too when they refuse a `:direct` flag on this port (`ADR-046` §7) —
        // one answer to *which port is routed*, so the port the proxy is handed
        // and the port kept on loopback cannot drift apart.
        let Some(port) = manifest.route_port() else {
            note(
                k,
                journal,
                Severity::Warn,
                "an application asked to be proxied and declares no TCP socket, so it has NO \
                 route. A path proxy forwards to a port; there is nothing to forward to",
                vec![
                    (
                        "application".to_string(),
                        Value::Str(name.as_str().to_string()),
                    ),
                    ("path".to_string(), Value::Str(path.to_string())),
                ],
            );
            continue;
        };
        wanted.push(rivet_core::proxy::Wanted {
            service: name.as_str().to_string(),
            path: path.to_string(),
            port,
            requires_operator: manifest.proxy_auth().requires_operator(),
            // **`ADR-048` §4's per-path rules, carried straight through.** The
            // supervisor decides nothing about them: it is the signed manifest
            // that says which prefix is open, and this function's whole claim is
            // that no column of the table comes from anywhere else.
            rules: manifest.proxy_auth_paths().to_vec(),
        });
    }

    let (routes, collisions) = rivet_core::proxy::build(&wanted);

    // **Both claimants lose, and the operator is told which two.** `ADR-015`
    // §6.1's rule pointed at a conflict: an operator whose application silently
    // lost a race cannot tell that is what happened.
    for collision in &collisions {
        note(
            k,
            journal,
            Severity::Warn,
            "two applications ask for one proxy path, so NEITHER is routed. Reachability that \
             depended on install order would be worse than no route at all; uninstall one",
            vec![
                ("path".to_string(), Value::Str(collision.path.clone())),
                (
                    "claimants".to_string(),
                    Value::Str(collision.claimants.join(", ")),
                ),
            ],
        );
    }

    // **A route that needs no operator is journalled by name.** `ADR-046` §4.3
    // makes it a signed declaration rather than a setting, so nobody can turn
    // it off on the machine — which makes *saying so, every convergence* the
    // only thing standing between a public route and a public route nobody
    // remembers agreeing to.
    for route in routes.iter().filter(|r| !r.requires_operator) {
        note(
            k,
            journal,
            Severity::Warn,
            "a route is served to callers presenting NO operator certificate, because its \
             signed manifest declares proxy-auth = \"none\" (ADR-046 §4.3). It cannot be turned \
             off on this machine: it takes a new artifact",
            vec![
                ("application".to_string(), Value::Str(route.service.clone())),
                ("path".to_string(), Value::Str(route.path.clone())),
            ],
        );
    }

    // **And a PREFIX served without an operator is journalled the same way** —
    // `ADR-048` §4. It is the same signed declaration made about part of an
    // application instead of all of one, so it earns the same warning: an open
    // prefix on an otherwise gated route is the shape nobody would notice, and
    // it is the shape a phone door takes on a service holding personal data.
    for route in &routes {
        let open = route.open_prefixes();
        if open.is_empty() {
            continue;
        }
        note(
            k,
            journal,
            Severity::Warn,
            "prefixes under a route are served to callers presenting NO operator certificate, \
             because the application's signed manifest declares them (ADR-048 §4). They cannot \
             be turned off on this machine: it takes a new artifact",
            vec![
                ("application".to_string(), Value::Str(route.service.clone())),
                (
                    "paths".to_string(),
                    Value::Str(
                        open.iter()
                            .map(|prefix| format!("/{}/{prefix}", route.path))
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                ),
            ],
        );
    }

    // **The administration portal, derived from the installed set exactly as
    // the routes are.** `rivet-fleetd` runs on a fleet SERVER and on no member,
    // declares no `proxy-path`, and is reached directly on its own port
    // (`ADR-046` §7) — so it is not a route, but its PRESENCE is a fact about
    // what is installed, and the proxy's page claims to be derived from that.
    // Carrying it here is what lets the proxy render its row only where there is
    // a fleet server to reach, instead of asserting one on every machine.
    let admin_portal = installed
        .iter()
        .any(|one| one.name.as_str() == rivet_core::proxy::ADMIN_PORTAL_SERVICE)
        .then(|| rivet_core::proxy::AdminPortal {
            service: rivet_core::proxy::ADMIN_PORTAL_SERVICE.to_string(),
            port: rivet_core::proxy::ADMIN_PORTAL_PORT,
        });

    // **Written even when it is empty**, for the settings file's reason: a
    // table left behind from a previous convergence is a route to an
    // application that has since been uninstalled.
    let text = rivet_core::proxy::render(&routes, admin_portal.as_ref());
    match k.write_file_nofollow(&destination, text.as_bytes(), rivet_core::proxy::MODE) {
        Ok(()) => note(
            k,
            journal,
            Severity::Info,
            "the proxy's routing table was derived from installed manifests and written",
            vec![
                (
                    "routes".to_string(),
                    Value::Int(i64::try_from(routes.len()).unwrap_or(0)),
                ),
                (
                    "collisions".to_string(),
                    Value::Int(i64::try_from(collisions.len()).unwrap_or(0)),
                ),
            ],
        ),
        Err(why) => note(
            k,
            journal,
            Severity::Warn,
            "the proxy's routing table could not be written, so the proxy serves whatever it \
             was handed last - or 404s everything. It is not stopped over this",
            vec![
                ("path".to_string(), Value::Str(destination)),
                ("error".to_string(), Value::Str(format!("{why:?}"))),
            ],
        ),
    }

    // **Computed regardless of whether the write above succeeded.** A failed
    // write already means the proxy serves a stale table until the next
    // successful derivation; what this answers is a fact about the set just
    // derived, for `reconcile_proxy_listen` to compare against what is
    // presently stored.
    rivet_core::proxy::any_plain_rule(&routes)
}

/// **Tells `rivet-proxyd`'s STORED `listen` what the table [`publish_routes`]
/// just derived wants** — `ADR-046` §7, `ADR-061` §7, `.rulings/` R-221. The
/// caller is every one of `publish_routes`' own call sites, right after it
/// returns.
///
/// # Never restarts, and never reopens a listener
///
/// `crate::supervise::Supervisor::set_proxy_listen` only changes what the
/// NEXT start of `rivet-proxyd` will see — see its own doc comment.
///
/// **Before the first convergence — every boot's first call — that next
/// start is the proxy's own first one**, so the very first listener set is
/// already correct with nothing further required: `main.rs` calls this
/// (through `publish_routes`) before `Supervisor::converge` ever runs.
///
/// **After the proxy has already started**, a change here is stored but not
/// LIVE until an operator restarts it. Whether a deploy or an install may
/// restart a service nobody asked it to is `ADR-055` §6's open question,
/// and this function does not decide it: it only journals what changed and
/// the exact command that applies it.
pub fn reconcile_proxy_listen(k: &mut dyn Kernel, sup: &mut crate::supervise::Supervisor, plain: bool) {
    use rivet_core::package::application::Socket;

    // **443 always; 80 only when the table just derived wants it** —
    // `ADR-046` §7's "the only thing that opens port 80". 443 is always
    // first, so `tcp-80`, when present, is always the SECOND descriptor —
    // the deterministic order `rivet-proxyd`'s own `LISTEN_NAME_PLAIN`
    // expects.
    let wanted = if plain {
        vec![Socket::Tcp(443), Socket::Tcp(80)]
    } else {
        vec![Socket::Tcp(443)]
    };
    // **Each `sup` call below is its own borrow, on purpose.** `journal_mut`
    // borrows the whole supervisor, so it is taken last and only when there
    // is something to say — holding it across `set_proxy_listen` or
    // `service_is_running` would be two live mutable borrows of `sup` at once.
    if !sup.set_proxy_listen(PROXY_SERVICE, wanted) {
        // Nothing installed as the proxy, or nothing about its stored listen
        // changed — the ordinary case on almost every pass.
        return;
    }

    if sup.service_is_running(PROXY_SERVICE) {
        let journal = sup.journal_mut();
        note(
            k,
            journal,
            Severity::Warn,
            if plain {
                "the routing table now carries a plain: rule, so rivet-proxyd's STORED listen \
                 gained port 80 - but the RUNNING proxy will not open it until it is restarted. \
                 Run: apply service-restart rivet-proxyd confirm"
            } else {
                "the routing table no longer carries a plain: rule, so rivet-proxyd's STORED \
                 listen dropped port 80 - but the RUNNING proxy keeps it open until it is \
                 restarted. Run: apply service-restart rivet-proxyd confirm"
            },
            vec![("plain".to_string(), Value::Bool(plain))],
        );
    }
    // **Not yet running: nothing to journal.** `Self::start` (`supervise.rs`)
    // reads the stored value fresh at every start, so the change is already
    // correct the first time this service is started.
}

/// **The firewall this machine's installed manifests call for** — `ADR-023`
/// §3 and §9.
///
/// Every application with a `proxy-path` has its routed port — the first TCP
/// `listen`, [`Manifest::route_port`](rivet_core::package::application::Manifest::route_port),
/// the same answer [`publish_routes`] hands the proxy — guarded, and two uids
/// are admitted to it: `rivet-proxyd`'s, which is the caller the port exists
/// for, and root's. **Root is admitted because refusing it would protect
/// nothing**: a root process can delete the table, and the control plane runs
/// as root and relays backups to an application's routed port (`ADR-051`).
///
/// **Every other port is untouched.** An application's further ports, its
/// `:direct` ports, and every port of an application with no `proxy-path` are
/// its own to declare and are not in the policy. **The control plane's port is
/// never in it**, even from a manifest that names it, so this rule cannot be
/// the thing that cuts the machine off.
///
/// A machine without the proxy installed gets an empty policy: the rule names
/// the proxy's uid, and with no proxy there is no caller to admit.
pub fn firewall_policy(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    installed: &[Installed],
) -> rivet_adapter_net::firewall::Policy {
    let Some(proxy) = installed.iter().find(|one| one.name.as_str() == PROXY_SERVICE) else {
        return rivet_adapter_net::firewall::Policy::new(&[], &[]);
    };
    let mut ports = Vec::new();
    for one in installed {
        let Some(manifest) = k
            .read_file(&rivet_core::service::manifest_path(&one.volume, &one.name))
            .ok()
            .and_then(|text| rivet_core::package::application::Manifest::parse(text.as_bytes()).ok())
        else {
            continue;
        };
        if manifest.proxy_path().is_none() {
            continue;
        }
        let Some(port) = manifest.route_port() else {
            continue;
        };
        if port == rivet_config::control_plane::DEFAULT_PORT {
            note(
                k,
                journal,
                Severity::Warn,
                "an application's routed port is the control plane's, so the firewall does NOT \
                 guard it. Nothing installed may narrow who reaches the control plane",
                vec![
                    ("application".to_string(), Value::Str(one.name.as_str().to_string())),
                    ("port".to_string(), Value::Int(i64::from(port))),
                ],
            );
            continue;
        }
        ports.push(port);
    }
    let proxy_uid = rivet_core::service::app_identity(&proxy.name).uid;
    rivet_adapter_net::firewall::Policy::new(&[proxy_uid, 0], &ports)
}

/// **Apply [`firewall_policy`] when it has changed since this process last
/// asked**, and journal what the kernel then holds — `ADR-023` §9.
///
/// Called where [`publish_routes`] is — at boot before the first convergence,
/// and after a finished install — and after an uninstall removes an
/// application, so the set of guarded ports follows what is installed.
///
/// # Nothing here is fatal, and nothing here can black-hole the machine
///
/// A kernel without nf_tables, a step it refuses, a read-back that does not
/// match: each is journalled and the machine goes on converging. A refused
/// transaction is applied by the kernel not at all, so filtering is what it
/// was before; and the chain's own policy is `accept`, so even a table that
/// is in place cannot refuse anything its rules do not name.
pub fn reconcile_firewall(k: &mut dyn Kernel, sup: &mut crate::supervise::Supervisor) {
    let installed = installed_applications(sup.services());
    let policy = firewall_policy(k, sup.journal_mut(), &installed);
    if !sup.take_firewall_change(&policy) {
        return;
    }
    let ports = policy
        .ports()
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let admitted = policy
        .admitted()
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let journal = sup.journal_mut();
    match k.apply_firewall(&policy) {
        Ok(applied) if applied.matches(&policy) => {
            note(
                k,
                journal,
                Severity::Info,
                if policy.ports().is_empty() {
                    "the firewall has nothing to guard: no proxied application is installed, so \
                     this machine holds no Rivet nftables table (read back from the kernel)"
                } else {
                    "the firewall is in place and was read back from the kernel: a new loopback \
                     connection to a proxied application's port is dropped unless it comes from \
                     the proxy or root (ADR-023)"
                },
                vec![
                    ("table".to_string(), Value::Str(format!("inet {}", rivet_adapter_net::firewall::TABLE))),
                    ("ports".to_string(), Value::Str(ports)),
                    ("admitted_uids".to_string(), Value::Str(admitted)),
                    (
                        "rules".to_string(),
                        Value::Str(
                            applied
                                .found
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join("; "),
                        ),
                    ),
                ],
            );
            // **The kernel's own bytes, once per change**, at debug: the
            // record a wire-format fixture is taken from.
            note(
                k,
                journal,
                Severity::Debug,
                "the firewall's read-back, as the kernel sent it",
                vec![(
                    "readback_hex".to_string(),
                    Value::Str(applied.raw.iter().map(|d| hex(d)).collect::<Vec<_>>().join(" ")),
                )],
            );
        }
        Ok(applied) => note(
            k,
            journal,
            Severity::Warn,
            "the firewall was accepted by the kernel and does NOT read back as asked, so what it \
             guards is what `rules` says and not what this machine's manifests call for",
            vec![
                ("ports".to_string(), Value::Str(ports)),
                ("admitted_uids".to_string(), Value::Str(admitted)),
                (
                    "rules".to_string(),
                    Value::Str(
                        applied
                            .found
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join("; "),
                    ),
                ),
                (
                    "unrecognised".to_string(),
                    Value::Int(i64::try_from(applied.unrecognised).unwrap_or(i64::MAX)),
                ),
            ],
        ),
        Err(why) => note(
            k,
            journal,
            Severity::Warn,
            "the firewall could NOT be applied, so loopback connections to proxied applications \
             are filtered as they were before this attempt. Networking and the control plane are \
             untouched",
            vec![
                ("ports".to_string(), Value::Str(ports)),
                ("error".to_string(), Value::Str(why)),
            ],
        ),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// **Derive what this machine's applications offer to back up, and publish it**
/// — `ADR-051`, `rivet_core::backup`.
///
/// # It is [`publish_routes`] a second time, deliberately
///
/// Every row comes from an installed application's **signed manifest** — its
/// `proxy-path`, and its `backup-path`/`backup-format` (with `restore-path`
/// when it accepts a restore, `ADR-051` §12) or its `backup-feed`.
/// Nothing here is configured. An operator cannot add a backup surface, cannot
/// move one, and cannot change what one claims to be: doing any of those means
/// shipping a new artifact that somebody signed.
///
/// # Where it goes, and why it is NOT beside an application
///
/// [`publish_routes`] writes into `rivet-proxyd`'s own tree because the proxy
/// is the reader and it is on this machine. **This table has no reader on the
/// machine at all** — its consumer is a workstation over `ADR-035`'s control
/// plane — so there is no application whose directory it belongs beside, and
/// picking one would make the file vanish when that one was uninstalled. It
/// goes to `rivet_core::backup::PATH` in `/run/rivet`, which is root-owned, so
/// the property the proxy table's placement was chosen for still holds: an
/// application cannot rewrite what it claims to offer.
///
/// # Nothing here is fatal
///
/// A manifest that will not parse, an application declaring a backup with no
/// route: each is skipped and none stops a convergence. `ADR-038` §4's
/// containment again — the machine must not stop converging over a backup
/// surface, because a machine that stops converging is one nothing can fix
/// remotely.
///
/// # A machine with no proxy still gets a table, and it is empty
///
/// Unlike [`publish_routes`], which writes nothing where the proxy is not
/// installed. That asymmetry is the placement: the proxy's table lives in the
/// proxy's tree and there is no tree without it, while this one lives in
/// `/run/rivet` and always has somewhere to go. An operator asking a machine
/// what it offers must be able to tell *"nothing"* from *"this build does not
/// answer that question"*, and an absent file cannot.
pub fn publish_backups(k: &mut dyn Kernel, journal: &mut Journal, installed: &[Installed]) {
    let mut wanted = Vec::new();
    for one in installed {
        let (volume, name) = (&one.volume, &one.name);
        let Some(manifest) = k
            .read_file(&rivet_core::service::manifest_path(volume, name))
            .ok()
            .and_then(|text| {
                rivet_core::package::application::Manifest::parse(text.as_bytes()).ok()
            })
        else {
            continue;
        };
        let Some(surface) = manifest.backup() else {
            continue;
        };
        // **A backup surface with no route is unreachable**, and the manifest
        // already refuses the pair — so reaching this means a manifest from a
        // build that did not. It is skipped rather than published at a route
        // this machine would have to invent.
        let Some(route) = manifest.proxy_path() else {
            note(
                k,
                journal,
                Severity::Warn,
                "an application declares a backup surface and no proxy-path, so it has NO \
                 published backup. A backup path is relative to the route the application is \
                 mounted at, and there is no route to be relative to",
                vec![(
                    "application".to_string(),
                    Value::Str(name.as_str().to_string()),
                )],
            );
            continue;
        };
        wanted.push(rivet_core::backup::Wanted {
            service: name.as_str().to_string(),
            route: route.to_string(),
            surface: surface.clone(),
        });
    }

    let declarations = rivet_core::backup::build(&wanted);

    // **Written even when it is empty**, for the routing table's reason: a
    // table left behind from a previous convergence names an application that
    // has since been uninstalled, and a client would fetch from a route that
    // is no longer served.
    let text = rivet_core::backup::render(&declarations);
    match k.write_file(
        rivet_core::backup::PATH,
        text.as_bytes(),
        rivet_core::backup::MODE,
    ) {
        Ok(()) => note(
            k,
            journal,
            Severity::Info,
            "what this machine's applications offer to back up was derived from installed \
             manifests and published",
            vec![
                (
                    "declarations".to_string(),
                    Value::Int(i64::try_from(declarations.len()).unwrap_or(0)),
                ),
                // `ADR-051` §12: how many of them also accept a restore. A
                // count beside the count, so a boot can be read for both
                // without reading the file.
                (
                    "restores".to_string(),
                    Value::Int(
                        i64::try_from(
                            declarations
                                .iter()
                                .filter(|d| d.surface.restore_path().is_some())
                                .count(),
                        )
                        .unwrap_or(0),
                    ),
                ),
            ],
        ),
        Err(why) => note(
            k,
            journal,
            Severity::Warn,
            "what this machine's applications offer to back up could not be published, so a \
             backup client reads whatever was there last - or nothing. It is not stopped over \
             this",
            vec![
                (
                    "path".to_string(),
                    Value::Str(rivet_core::backup::PATH.to_string()),
                ),
                ("error".to_string(), Value::Str(format!("{why:?}"))),
            ],
        ),
    }
}

/// **Publish the display window** — `ADR-039` §3.1, `rivet_core::display`.
///
/// Called once per convergence beside `crate::etc::project`, from the same
/// document, so the two are consistent by construction rather than by ordering.
pub fn publish_display_hours(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    document: &rivet_config::Document,
) -> Published {
    let path = rivet_core::display::HOURS_PATH;
    let Some(hours) = document.display() else {
        // **Removed rather than left stale.** `remove_file` on a path that is
        // not there is the ordinary case on every boot of every machine that
        // has never set the key, so it is not reported as a failure.
        return match k.remove_file(path) {
            Ok(()) | Err(Errno::ENOENT) => Published::Withdrew,
            Err(why) => {
                note(
                    k,
                    journal,
                    Severity::Warn,
                    "the display window could not be withdrawn, so a screen may still go dark \
                     on a schedule the document no longer carries",
                    vec![
                        ("path".to_string(), Value::Str(path.to_string())),
                        ("error".to_string(), Value::Str(format!("{why:?}"))),
                    ],
                );
                Published::Failed
            }
        };
    };
    // Two fields, `off` first, in the machine's local time. The format and its
    // reader are `rivet_sdk::schedule::Hours::parse`, and the agreement test in
    // that crate is what keeps the two honest.
    let contents = format!(
        "# Generated by Rivet from the `[display]` table of the configuration document.\n\
         # Do not edit: this is a view of the document, never an interface (ADR-007 §3.6.2).\n\
         # The hours this screen is DARK, off first, in this machine's local time.\n\
         {} {}\n",
        hours.off(),
        hours.on()
    );
    match k.write_file(path, contents.as_bytes(), MODE) {
        Ok(()) => Published::Wrote,
        Err(why) => {
            note(
                k,
                journal,
                Severity::Warn,
                "the display window could not be published, so nothing holding the screen can \
                 know when it is due to go dark",
                vec![
                    ("path".to_string(), Value::Str(path.to_string())),
                    ("error".to_string(), Value::Str(format!("{why:?}"))),
                ],
            );
            Published::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    // A test asserts; `unwrap`, `expect` and `panic!` ARE the assertion.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    use crate::testkernel::FakeKernel;
    use rivet_core::package::application::{Manifest, ProgramKind, ProxyAuth, Socket};

    fn journal() -> Journal {
        Journal::new("boot".to_string(), 0, SELF_NAME.to_string())
    }

    fn document_with(off: &str, on: &str) -> rivet_config::Document {
        let toml = format!("version = 1\n[display]\noff = \"{off}\"\non = \"{on}\"\n");
        rivet_config::Document::parse(&toml).expect("a valid document")
    }

    // -----------------------------------------------------------------------
    // The proxy's routing table — ADR-046 §5, R-133, R-152
    // -----------------------------------------------------------------------

    /// The bytes of a manifest asking for what the arguments say.
    fn app_manifest(path: Option<&str>, auth: ProxyAuth, listen: Vec<Socket>) -> String {
        Manifest::new(ProgramKind::Daemon)
            .with_listen(listen)
            .expect("a short list")
            .with_proxy_path(path.map(ToString::to_string))
            .expect("a legal segment")
            .with_proxy_auth(auth)
            .render()
    }

    /// Put an application on the fake machine's card, with that manifest.
    fn install_app(k: &mut FakeKernel, app: &str, manifest: &str) {
        k.files_mut().insert(
            rivet_core::service::manifest_path(
                rivet_core::service::BUILT_IN_VOLUME,
                &rivet_core::service::ServiceName::new(app).expect("a name"),
            ),
            crate::testkernel::Script::always(Ok(manifest.to_string())),
        );
    }

    fn app_list(names: &[&str]) -> Vec<Installed> {
        names
            .iter()
            .map(|name| Installed {
                name: rivet_core::service::ServiceName::new(name).expect("a name"),
                volume: rivet_core::service::BUILT_IN_VOLUME.to_string(),
            })
            .collect()
    }

    /// What was written to the proxy's table, if anything was.
    fn table(k: &FakeKernel) -> Option<String> {
        let want = rivet_core::proxy::path(
            rivet_core::service::BUILT_IN_VOLUME,
            &rivet_core::service::ServiceName::new("rivet-proxyd").expect("a name"),
        );
        k.writes
            .iter()
            .find(|w| w.path == want)
            .map(|w| String::from_utf8(w.contents.clone()).expect("utf-8"))
    }

    /// **The whole of `ADR-046` §5 from the outside**: every column comes from
    /// a signed manifest, and the gate column comes from `proxy-auth`.
    #[test]
    fn the_table_is_derived_from_manifests_and_carries_the_gate() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install_app(
            &mut k,
            "rivet-proxyd",
            &app_manifest(None, ProxyAuth::Operator, vec![Socket::Tcp(443)]),
        );
        install_app(
            &mut k,
            "notes-rivet",
            &app_manifest(Some("notes"), ProxyAuth::Operator, vec![Socket::Tcp(8082)]),
        );
        install_app(
            &mut k,
            "tempmon-rivet",
            &app_manifest(Some("tempmon"), ProxyAuth::None, vec![Socket::Tcp(8081)]),
        );

        publish_routes(
            &mut k,
            &mut j,
            &app_list(&["rivet-proxyd", "notes-rivet", "tempmon-rivet"]),
        );

        let text = table(&k).expect("no table was written");
        let routes = rivet_core::proxy::parse(&text);
        assert_eq!(routes.len(), 2, "{text}");
        let notes = routes.iter().find(|r| r.path == "notes").expect("notes");
        assert_eq!(notes.port, 8082);
        assert!(
            notes.requires_operator,
            "a manifest that says nothing about callers must be gated"
        );
        let tempmon = routes.iter().find(|r| r.path == "tempmon").expect("tempmon");
        assert!(
            !tempmon.requires_operator,
            "R-152's TempMon declared proxy-auth = \"none\" and must be public"
        );
        // The proxy itself asked for no path, so it is not a route to itself.
        assert!(!routes.iter().any(|r| r.service == "rivet-proxyd"), "{text}");
    }

    /// **A public route is journalled every time it is derived.** It is a
    /// signed declaration, so nobody can turn it off on the machine — which
    /// makes saying so the only thing between a public route and a public route
    /// nobody remembers agreeing to.
    #[test]
    fn a_route_that_needs_no_operator_is_reported_by_name() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install_app(
            &mut k,
            "rivet-proxyd",
            &app_manifest(None, ProxyAuth::Operator, vec![Socket::Tcp(443)]),
        );
        install_app(
            &mut k,
            "tempmon-rivet",
            &app_manifest(Some("tempmon"), ProxyAuth::None, vec![Socket::Tcp(8081)]),
        );
        publish_routes(&mut k, &mut j, &app_list(&["rivet-proxyd", "tempmon-rivet"]));

        let said: Vec<String> = j.pending().iter().map(|r| r.msg.clone()).collect();
        assert!(
            said.iter().any(|m| m.contains("NO operator certificate")),
            "a public route was not reported: {said:?}"
        );
    }

    /// **A machine that carries no proxy gets no file**, which is most Rivets.
    /// Writing one would put a file in a tree nothing owns.
    #[test]
    fn a_machine_without_the_proxy_has_no_table_written() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install_app(
            &mut k,
            "notes-rivet",
            &app_manifest(Some("notes"), ProxyAuth::Operator, vec![Socket::Tcp(8082)]),
        );
        publish_routes(&mut k, &mut j, &app_list(&["notes-rivet"]));
        assert!(table(&k).is_none());
        assert!(k.writes.is_empty(), "{:?}", k.writes);
    }

    /// A path proxy forwards to a port. An application that asked to be proxied
    /// and declared no TCP socket gets NO route and is told so by name —
    /// omitting the row silently would leave its author with a route that 404s
    /// and nothing saying why.
    #[test]
    fn asking_for_a_path_with_no_tcp_socket_is_refused_by_name() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install_app(
            &mut k,
            "rivet-proxyd",
            &app_manifest(None, ProxyAuth::Operator, vec![Socket::Tcp(443)]),
        );
        install_app(
            &mut k,
            "odd-app",
            &app_manifest(
                Some("odd"),
                ProxyAuth::Operator,
                vec![Socket::Unix("api".to_string())],
            ),
        );
        publish_routes(&mut k, &mut j, &app_list(&["rivet-proxyd", "odd-app"]));

        let text = table(&k).expect("the file is still written");
        assert!(rivet_core::proxy::parse(&text).is_empty(), "{text}");
        let said: Vec<String> = j.pending().iter().map(|r| r.msg.clone()).collect();
        assert!(
            said.iter().any(|m| m.contains("declares no TCP socket")),
            "nothing named the application: {said:?}"
        );
    }

    /// **Both claimants lose and the operator is told which two.**
    #[test]
    fn two_applications_asking_for_one_path_get_no_route_and_are_named() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install_app(
            &mut k,
            "rivet-proxyd",
            &app_manifest(None, ProxyAuth::Operator, vec![Socket::Tcp(443)]),
        );
        for app in ["aura-controller", "aura-renderer"] {
            install_app(
                &mut k,
                app,
                &app_manifest(Some("aura"), ProxyAuth::Operator, vec![Socket::Tcp(8080)]),
            );
        }
        publish_routes(
            &mut k,
            &mut j,
            &app_list(&["rivet-proxyd", "aura-controller", "aura-renderer"]),
        );

        let text = table(&k).expect("the file is still written");
        assert!(rivet_core::proxy::parse(&text).is_empty(), "{text}");
        let said: Vec<String> = j.pending().iter().map(|r| r.msg.clone()).collect();
        assert!(
            said.iter().any(|m| m.contains("two applications ask for one proxy path")
                || m.contains("NEITHER is routed")),
            "the collision was not reported: {said:?}"
        );
    }

    /// **Written even when it is empty**, for the settings file's reason: a
    /// table left from a previous convergence is a route to an application that
    /// has since been uninstalled.
    #[test]
    fn a_machine_with_the_proxy_and_no_routes_still_gets_an_empty_table() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install_app(
            &mut k,
            "rivet-proxyd",
            &app_manifest(None, ProxyAuth::Operator, vec![Socket::Tcp(443)]),
        );
        publish_routes(&mut k, &mut j, &app_list(&["rivet-proxyd"]));
        assert_eq!(table(&k).as_deref(), Some(""));
    }

    /// **The table is written through the GUARDED write.** It addresses a
    /// root-owned directory today (`rivet_core::proxy::path` moved it there
    /// deliberately), so the guards are belt and braces — and this is the
    /// tripwire for an edit that moves it back into `data/`, which the
    /// application owns and can plant a link in.
    #[test]
    fn the_routing_table_is_written_with_the_symlink_guards() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install_app(
            &mut k,
            "rivet-proxyd",
            &app_manifest(None, ProxyAuth::Operator, vec![Socket::Tcp(443)]),
        );
        publish_routes(&mut k, &mut j, &app_list(&["rivet-proxyd"]));
        let expected = rivet_core::proxy::path(
            rivet_core::service::BUILT_IN_VOLUME,
            &rivet_core::service::ServiceName::new("rivet-proxyd").expect("a name"),
        );
        assert!(
            k.nofollow_writes.contains(&expected),
            "the routing table was written with the ordinary write: guarded writes were {:?}",
            k.nofollow_writes
        );
    }

    // -----------------------------------------------------------------------
    // Settings delivery — ADR-038 §6
    // -----------------------------------------------------------------------

    fn document_carrying(table: &str) -> rivet_config::Document {
        rivet_config::Document::parse(&format!("version = 1\n{table}"))
            .expect("a valid document")
    }

    fn installed(
        name: &str,
        declared: &[&str],
    ) -> Vec<(rivet_core::service::ServiceName, String, Vec<String>)> {
        vec![(
            rivet_core::service::ServiceName::new(name).expect("a name"),
            rivet_core::service::BUILT_IN_VOLUME.to_string(),
            declared.iter().map(|s| (*s).to_string()).collect(),
        )]
    }

    fn delivered(k: &FakeKernel, name: &str) -> Option<String> {
        let want = rivet_core::settings::path(
            rivet_core::service::BUILT_IN_VOLUME,
            &rivet_core::service::ServiceName::new(name).expect("a name"),
        );
        k.writes
            .iter()
            .find(|w| w.path == want)
            .map(|w| String::from_utf8(w.contents.clone()).expect("utf-8"))
    }

    // -----------------------------------------------------------------------
    // The name service's projection — ADR-047 §6
    // -----------------------------------------------------------------------

    fn projection_path() -> String {
        rivet_core::name_service::path(
            rivet_core::service::BUILT_IN_VOLUME,
            &rivet_core::service::ServiceName::new(rivet_core::service::DNSD_SERVICE_NAME)
                .expect("a name"),
        )
    }

    /// **The name service is handed its table and the machine's name**,
    /// root-owned and readable, with the guarded write, and as a document the
    /// service's own parser reads back.
    #[test]
    fn the_name_service_is_handed_its_table_and_the_machines_name() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        let doc = document_carrying(
            "[applications.photo-index]\nalbum = \"holidays\"\n\
             [dns]\nzone = \"f.home.arpa\"\n[dns.records.printer]\naddress = \"10.0.0.40\"\n",
        );
        deliver_name_service_document(
            &mut k,
            &mut j,
            &doc,
            Some("rivet-06f5c"),
            &installed(rivet_core::service::DNSD_SERVICE_NAME, &[]),
        );
        let want = projection_path();
        let write = k.writes.iter().find(|w| w.path == want).expect("the projection was written");
        assert_eq!(write.mode, 0o644);
        assert!(k.nofollow_writes.contains(&want), "written without the guards");
        let text = String::from_utf8(write.contents.clone()).expect("utf-8");
        let back = rivet_config::Document::parse(&text).expect("the projection is a document");
        assert_eq!(back.hostname().map(rivet_core::hostname::Hostname::as_str), Some("rivet-06f5c"));
        assert_eq!(back.dns(), doc.dns());
        assert!(!text.contains("photo-index"), "an application's settings leaked: {text}");
    }

    /// A machine that does not run the name service — it is not in the list,
    /// which holds an image service only while it is enabled — gets no file.
    #[test]
    fn a_machine_not_running_the_name_service_is_handed_no_projection() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        deliver_name_service_document(
            &mut k,
            &mut j,
            &document_carrying("[dns]\nzone = \"f.home.arpa\"\n"),
            Some("rivet-06f5c"),
            &installed("rivet-proxyd", &["upstream-name"]),
        );
        assert!(k.writes.is_empty(), "{:?}", k.writes.iter().map(|w| &w.path).collect::<Vec<_>>());
    }

    /// A machine `crate::naming` could not name projects no name, so the
    /// service refuses rather than serving a zone whose SOA names nobody.
    #[test]
    fn a_nameless_machine_projects_no_hostname() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        deliver_name_service_document(
            &mut k,
            &mut j,
            &document_carrying("hostname = \"kitchen-pi\"\n[dns]\nzone = \"f.home.arpa\"\n"),
            None,
            &installed(rivet_core::service::DNSD_SERVICE_NAME, &[]),
        );
        let want = projection_path();
        let write = k.writes.iter().find(|w| w.path == want).expect("the projection was written");
        let back = rivet_config::Document::parse(std::str::from_utf8(&write.contents).expect("utf-8"))
            .expect("a document");
        assert_eq!(back.hostname(), None);
    }

    // -----------------------------------------------------------------------
    // The mDNS responder's projection — ADR-017 §3
    // -----------------------------------------------------------------------

    fn mdns_path() -> String {
        rivet_core::mdns::path(
            rivet_core::service::BUILT_IN_VOLUME,
            &rivet_core::service::ServiceName::new(rivet_core::service::MDNSD_SERVICE_NAME)
                .expect("a name"),
        )
    }

    fn responder_inputs() -> crate::mdns::Inputs {
        crate::mdns::Inputs {
            hostname: Some("rivet-06f5c".to_string()),
            addressed: vec![("eth0".to_string(), std::net::Ipv4Addr::new(192, 168, 50, 66))],
            leaseless: vec!["wlan0".to_string()],
        }
    }

    /// **The responder is handed exactly the inputs the supervisor compares**,
    /// root-owned and readable, with the guarded write, in the format its own
    /// reader takes back.
    #[test]
    fn the_mdns_responder_is_handed_the_inputs_the_supervisor_compares() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        deliver_mdns_projection(
            &mut k,
            &mut j,
            Some(&responder_inputs()),
            &installed(rivet_core::service::MDNSD_SERVICE_NAME, &[]),
        );
        let want = mdns_path();
        let write = k.writes.iter().find(|w| w.path == want).expect("the projection was written");
        assert_eq!(write.mode, 0o644);
        assert!(k.nofollow_writes.contains(&want), "written without the guards");
        let back = rivet_core::mdns::Projection::parse(
            std::str::from_utf8(&write.contents).expect("utf-8"),
        )
        .expect("a complete projection");
        assert_eq!(back, responder_inputs().projection());
        assert_eq!(back.hostname.as_ref().map(rivet_core::hostname::Hostname::as_str), Some("rivet-06f5c"));
    }

    /// A machine that does not run the responder gets no file.
    #[test]
    fn a_machine_not_running_the_mdns_responder_is_handed_no_projection() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        deliver_mdns_projection(
            &mut k,
            &mut j,
            Some(&responder_inputs()),
            &installed(rivet_core::service::DNSD_SERVICE_NAME, &[]),
        );
        assert!(k.writes.is_empty(), "{:?}", k.writes.iter().map(|w| &w.path).collect::<Vec<_>>());
    }

    /// **What the document says and the manifest declares is handed over.**
    #[test]
    fn a_declared_setting_reaches_the_applications_own_directory() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        deliver_settings(
            &mut k,
            &mut j,
            &document_carrying("[applications.aura-renderer]\nbrightness = \"60\"\n"),
            &installed("aura-renderer", &["brightness"]),
        );
        let text = delivered(&k, "aura-renderer").expect("nothing was handed over");
        assert_eq!(
            rivet_core::settings::parse(&text),
            vec![("brightness".to_string(), "60".to_string())]
        );
    }

    /// **The check the declaration exists for.** Without it a misspelled key is
    /// accepted, stored, provisioned to a fleet, and produces an application
    /// that behaves as though it were never set — with nothing reporting it.
    #[test]
    fn a_key_the_manifest_does_not_declare_is_not_delivered_and_is_reported() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        deliver_settings(
            &mut k,
            &mut j,
            &document_carrying("[applications.aura-renderer]\nbrightnes = \"60\"\n"),
            &installed("aura-renderer", &["brightness"]),
        );
        let text = delivered(&k, "aura-renderer").expect("a file is still written");
        assert!(
            rivet_core::settings::parse(&text).is_empty(),
            "an undeclared key was handed over: {text}"
        );
        let said: Vec<String> = j.pending().iter().map(|r| r.msg.clone()).collect();
        assert!(
            said.iter().any(|m| m.contains("NOT delivered")),
            "nothing reported the misspelling: {said:?}"
        );
    }

    /// **A file is written even when nothing is set**, because an application
    /// whose settings were removed must see them gone — a file left from a
    /// previous document would be a value nobody has asked for since.
    #[test]
    fn removing_every_setting_still_writes_the_file() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        deliver_settings(
            &mut k,
            &mut j,
            &document_carrying(""),
            &installed("aura-renderer", &["brightness"]),
        );
        let text = delivered(&k, "aura-renderer").expect("the file must still be written");
        assert!(rivet_core::settings::parse(&text).is_empty());
    }

    /// **The settings file is written through the GUARDED write, and it is the
    /// only write in this daemon that must be.**
    ///
    /// `PLAN.md` §123.2 item 2. Every other `Kernel::write_file` here addresses
    /// a root-owned directory; this one addresses `data/`, which the
    /// application owns and can put a symlink in. What the guards do is proved
    /// against a real filesystem in `kernel`'s
    /// `the_guarded_write_refuses_a_symlink_a_hard_link_and_a_fifo`; what THIS
    /// asserts is that they are asked for at all, which is the half that a
    /// future edit to this function could silently drop.
    #[test]
    fn an_applications_settings_are_written_with_the_symlink_guards() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        deliver_settings(
            &mut k,
            &mut j,
            &document_carrying("[applications.aura-renderer]\nbrightness = \"60\"\n"),
            &installed("aura-renderer", &["brightness"]),
        );
        let expected = rivet_core::settings::path(
            rivet_core::service::BUILT_IN_VOLUME,
            &rivet_core::service::ServiceName::new("aura-renderer").expect("a name"),
        );
        assert!(
            k.nofollow_writes.contains(&expected),
            "the settings file was written with the ordinary write, which follows symlinks: \
             guarded writes were {:?}",
            k.nofollow_writes
        );
    }

    /// **A table for an application this machine has not installed is not an
    /// error.** Provisioning a fleet before the software arrives is the case
    /// the region exists for (`ADR-038` §4.1), so it is noted and kept.
    #[test]
    fn settings_for_an_application_that_is_not_installed_are_noted_not_refused() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        deliver_settings(
            &mut k,
            &mut j,
            &document_carrying("[applications.not-yet]\nk = \"v\"\n"),
            &installed("aura-renderer", &["brightness"]),
        );
        let said: Vec<String> = j.pending().iter().map(|r| r.msg.clone()).collect();
        assert!(
            said.iter().any(|m| m.contains("has not installed")),
            "an uninstalled application was not mentioned: {said:?}"
        );
        assert!(delivered(&k, "not-yet").is_none(), "a file was written for it");
    }

    /// What `rivet-config` could not carry at all is reported here, because
    /// this is the first place with a journal — `ADR-038` §4.4.
    #[test]
    fn what_the_document_could_not_carry_is_reported_once_it_reaches_a_journal() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        deliver_settings(
            &mut k,
            &mut j,
            &document_carrying("[applications.aura-renderer]\nbrightness = 60\n"),
            &installed("aura-renderer", &["brightness"]),
        );
        let said: Vec<String> = j.pending().iter().map(|r| r.msg.clone()).collect();
        assert!(
            said.iter().any(|m| m.contains("could not be carried")),
            "a non-text value was dropped in silence: {said:?}"
        );
    }

    /// **The machine publishes what it is**, so nothing has to assume it was
    /// built alongside the image.
    ///
    /// The version written is THIS BINARY's, and that is the property under
    /// test: the supervisor ships inside the image, so the two cannot drift.
    /// `cmd/rivet-launcher` is the program that assumed its own version was the
    /// machine's and drew `Rivet OS 0.1.61` on a 0.1.63 board for days.
    #[test]
    fn the_machine_publishes_the_version_of_the_image_it_is() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        publish_os_version(&mut k, &mut j);

        let write = k
            .writes
            .iter()
            .find(|w| w.path == rivet_core::generated::OS_VERSION_PATH)
            .expect("the version should have been published");
        assert_eq!(write.mode, 0o644, "an application could not read it");
        let text = String::from_utf8(write.contents.clone()).expect("utf-8");
        assert_eq!(
            text.trim(),
            env!("CARGO_PKG_VERSION"),
            "the supervisor must publish ITS OWN version - it is the one that ships in the image"
        );
        // Newline-terminated, because a reader trims and a file that is a bare
        // token is a file somebody will `cat` and get no prompt back from.
        assert!(text.ends_with('\n'), "{text:?}");
    }

    /// It goes to /run/rivet and not /etc, for the display window's reason:
    /// no C runtime asks what version of Rivet this is.
    #[test]
    fn the_version_is_published_into_rivets_own_runtime_namespace() {
        let path = rivet_core::generated::OS_VERSION_PATH;
        assert!(path.starts_with("/run/rivet/"), "{path}");
        assert!(!path.starts_with("/etc/"));
    }

    /// **The path is not under `/etc`**, asserted rather than left to the module
    /// note — the first build of this put it there and the maintainer caught it.
    #[test]
    fn the_window_is_published_into_rivets_own_runtime_namespace() {
        let path = rivet_core::display::HOURS_PATH;
        assert!(path.starts_with("/run/rivet/"), "{path}");
        assert!(
            !path.starts_with("/etc/"),
            "/etc is for paths a C runtime demands, and none demands this one"
        );
    }

    /// A document with a window writes one file, readable by an unprivileged
    /// application — which is the whole point of publishing it.
    #[test]
    fn a_document_with_a_window_writes_it_where_an_application_can_read_it() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        let outcome = publish_display_hours(&mut k, &mut j, &document_with("19:00", "07:00"));
        assert_eq!(outcome, Published::Wrote);

        let write = k
            .writes
            .iter()
            .find(|w| w.path == rivet_core::display::HOURS_PATH)
            .expect("the window should have been written");
        assert_eq!(write.mode, 0o644, "an application could not read it");
        let text = String::from_utf8(write.contents.clone()).expect("utf-8");
        assert!(
            text.lines().any(|l| l == "19:00 07:00"),
            "the window is not in the file: {text:?}"
        );
        // The provenance header, so a person who runs `cat` on it learns what
        // it is and what to change instead - ADR-007 §3.6.1.
        assert!(text.starts_with("# Generated by Rivet from"), "{text:?}");
        assert!(text.contains("never an interface"), "{text:?}");
    }

    /// **The file the supervisor writes is the file `rivet_sdk` parses.** The
    /// SDK cannot be linked from the operating system, so the format is
    /// asserted here in the shape `rivet_sdk::schedule::Hours::parse` accepts:
    /// comments skipped, first real line, two whitespace-separated `HH:MM`.
    #[test]
    fn the_written_format_is_the_one_the_sdk_reader_expects() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        publish_display_hours(&mut k, &mut j, &document_with("02:00", "07:00"));
        let text = String::from_utf8(
            k.writes
                .iter()
                .find(|w| w.path == rivet_core::display::HOURS_PATH)
                .expect("written")
                .contents
                .clone(),
        )
        .expect("utf-8");

        let line = text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('#'))
            .expect("a line that is not a comment");
        let (off, on) = line.split_once(char::is_whitespace).expect("two fields");
        assert_eq!(
            rivet_core::display::TimeOfDay::parse(off.trim()),
            rivet_core::display::TimeOfDay::new(2, 0)
        );
        assert_eq!(
            rivet_core::display::TimeOfDay::parse(on.trim()),
            rivet_core::display::TimeOfDay::new(7, 0)
        );
    }

    /// **A document that loses its window loses the file.** The failure this
    /// prevents is a building whose screens keep going dark on a schedule
    /// nobody can find in the document any more.
    #[test]
    fn removing_the_table_withdraws_the_file_rather_than_leaving_it_stale() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        let outcome = publish_display_hours(&mut k, &mut j, &rivet_config::Document::default());
        assert_eq!(outcome, Published::Withdrew);
        assert!(
            k.removed.contains(&rivet_core::display::HOURS_PATH.to_string()),
            "the file was not withdrawn: {:?}",
            k.removed
        );
        assert!(
            !k.writes
                .iter()
                .any(|w| w.path == rivet_core::display::HOURS_PATH),
            "a machine with no window wrote one anyway"
        );
    }

    // -----------------------------------------------------------------------
    // The proxy as an image service — ADR-060
    // -----------------------------------------------------------------------

    /// A supervisor over the image's own services, with the proxy `enabled` or
    /// not, and `notes-rivet` installed behind it.
    fn image_proxy_machine(k: &mut FakeKernel, enabled: bool) -> crate::supervise::Supervisor {
        let mut defs = crate::definitions::system_services();
        for d in &mut defs {
            if d.name == rivet_core::service::PROXYD_SERVICE_NAME {
                d.enabled = enabled;
            }
        }
        let manifest = app_manifest(Some("notes"), ProxyAuth::Operator, vec![Socket::Tcp(8082)]);
        install_app(k, "notes-rivet", &manifest);
        defs.push(crate::applications::definition(
            rivet_core::service::BUILT_IN_VOLUME,
            &rivet_core::service::ServiceName::new("notes-rivet").expect("a name"),
            &Manifest::parse(manifest.as_bytes()).expect("a manifest"),
            &[],
        ));
        match crate::supervise::Supervisor::new(defs, crate::service::Limits::default(), journal()) {
            Ok(s) => s,
            Err(e) => panic!("fixture refused: {e}"),
        }
    }

    /// **The routing table reaches the image's proxy with no proxy application
    /// installed** — `ADR-060`. The destination is the image service's home,
    /// and the route is the installed application's.
    #[test]
    fn the_routing_table_is_written_for_the_image_proxy_with_no_proxy_application() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        let sup = image_proxy_machine(&mut k, true);
        publish_routes(&mut k, &mut j, &installed_applications(sup.services()));
        let text = table(&k).expect("no table was written for the image's proxy");
        let routes = rivet_core::proxy::parse(&text);
        assert_eq!(routes.len(), 1, "{text}");
        assert_eq!(routes[0].service, "notes-rivet");
    }

    /// **`upstream-name` reaches the image proxy's settings file**, declared by
    /// the image's table because it has no manifest to declare it in.
    #[test]
    fn upstream_name_reaches_the_image_proxys_settings_file() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        let sup = image_proxy_machine(&mut k, true);
        let declared = declared_settings(&mut k, sup.services());
        let proxy = declared
            .iter()
            .find(|(name, _, _)| name.as_str() == rivet_core::service::PROXYD_SERVICE_NAME)
            .expect("the image's proxy declares its settings");
        assert_eq!(
            proxy.2,
            vec![
                "upstream-name".to_string(),
                "operator-certificate".to_string(),
                "handshake-seconds".to_string()
            ]
        );

        deliver_settings(
            &mut k,
            &mut j,
            &document_carrying(
                "[applications.rivet-proxyd]\nupstream-name = \"rivet-01.local\"\n\
                 operator-certificate = \"refuse\"\nhandshake-seconds = \"30\"\n",
            ),
            &declared,
        );
        let text = delivered(&k, "rivet-proxyd").expect("no settings file for the proxy");
        assert!(text.contains("rivet-01.local"), "{text}");
        // **The certificate's retirement reaches the proxy by the same file**
        // (`.rulings/` R-234), and a document that says nothing writes no line.
        assert!(text.contains("operator-certificate=refuse"), "{text}");
        // So does the handshake deadline (`ADR-046` §10).
        assert!(text.contains("handshake-seconds=30"), "{text}");
        assert!(
            !j.pending().iter().any(|r| r.msg.contains("NOT delivered")),
            "the declared key was refused"
        );
    }

    /// **A proxy the document leaves off gets no table and no settings file**:
    /// it has no home to write them into, and on a machine without `/state`
    /// the write would fail on every boot about a service nobody asked for.
    #[test]
    fn a_disabled_image_proxy_is_written_nothing() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        let sup = image_proxy_machine(&mut k, false);
        publish_routes(&mut k, &mut j, &installed_applications(sup.services()));
        assert!(table(&k).is_none(), "a table was written for a proxy that is off");
        let declared = declared_settings(&mut k, sup.services());
        assert!(
            !declared
                .iter()
                .any(|(name, _, _)| name.as_str() == rivet_core::service::PROXYD_SERVICE_NAME),
            "settings were declared for a proxy that is off"
        );
    }

    // -----------------------------------------------------------------------
    // `rivet-proxyd`'s stored listen and port 80 — ADR-046 §7, ADR-061 §7,
    // `.rulings/` R-221
    // -----------------------------------------------------------------------

    /// A manifest asking for a route with per-path rules — `ADR-048` §4's
    /// shape, for a `plain:` prefix.
    fn app_manifest_with_rules(path: &str, rules: &[&str], listen: Vec<Socket>) -> String {
        let rules: Vec<_> = rules
            .iter()
            .map(|r| {
                rivet_core::package::application::ProxyRule::parse(r)
                    .expect("a rule this test meant to write")
            })
            .collect();
        Manifest::new(ProgramKind::Daemon)
            .with_listen(listen)
            .expect("a short list")
            .with_proxy_path(Some(path.to_string()))
            .expect("a legal segment")
            .with_proxy_auth_paths(rules)
            .expect("valid")
            .render()
    }

    /// The stored `listen` `rivet-proxyd` presently carries, as the socket
    /// names `Kernel::open_listeners` would be handed at its next start —
    /// `render_sockets`' question, asked of the supervisor's own definition
    /// rather than of a live descriptor.
    fn proxy_listen_names(sup: &crate::supervise::Supervisor) -> Vec<String> {
        sup.services()
            .iter()
            .find(|m| m.def.name == rivet_core::service::PROXYD_SERVICE_NAME)
            .map(|m| {
                m.def
                    .listen
                    .iter()
                    .map(|s| match s {
                        Socket::Tcp(p) | Socket::TcpDirect(p) => format!("tcp-{p}"),
                        Socket::Udp(p) | Socket::UdpDirect(p) => format!("udp-{p}"),
                        Socket::Unix(name) => name.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// **No `plain:` rule anywhere in the derived table: the proxy's stored
    /// listen never gains port 80** — the ordinary machine.
    #[test]
    fn no_plain_rule_leaves_the_proxys_stored_listen_at_443_only() {
        let mut k = FakeKernel::new();
        let mut sup = image_proxy_machine(&mut k, true);
        let installed = app_list(&["rivet-proxyd", "notes-rivet"]);
        let plain = publish_routes(&mut k, sup.journal_mut(), &installed);
        assert!(!plain, "notes declares no proxy-auth-paths rule at all");
        reconcile_proxy_listen(&mut k, &mut sup, plain);
        assert_eq!(proxy_listen_names(&sup), vec!["tcp-443".to_string()]);
    }

    /// **A `plain:` rule present before the FIRST convergence: the proxy's
    /// stored listen already carries port 80 by the time it starts, and
    /// `tcp-80` is handed over SECOND — a deterministic order.**
    ///
    /// `cmd/rivet-sysd/src/main.rs` calls `publish_routes` and
    /// `reconcile_proxy_listen` before `Supervisor::converge` ever runs on a
    /// boot — confirmed by reading it: `Supervisor::new` at ~L559,
    /// `publish_routes` at ~L968, the first `sup.converge(k)` at ~L1106, with
    /// nothing that starts a service between the first two. This test drives
    /// the same order over a `FakeKernel`.
    #[test]
    fn a_plain_rule_present_at_boot_is_bound_and_handed_as_tcp_80_second() {
        let mut k = FakeKernel::new();
        let mut sup = image_proxy_machine(&mut k, true);
        install_app(
            &mut k,
            "gantry-rivet",
            &app_manifest_with_rules("gantry", &["plain:install"], vec![Socket::Tcp(8080)]),
        );
        let installed = app_list(&["rivet-proxyd", "notes-rivet", "gantry-rivet"]);
        let plain = publish_routes(&mut k, sup.journal_mut(), &installed);
        assert!(plain);

        // Before the first convergence — the boot order `main.rs` uses.
        reconcile_proxy_listen(&mut k, &mut sup, plain);
        assert_eq!(
            proxy_listen_names(&sup),
            vec!["tcp-443".to_string(), "tcp-80".to_string()]
        );

        sup.converge(&mut k);
        assert_eq!(
            k.listeners_open
                .get(rivet_core::service::PROXYD_SERVICE_NAME)
                .map(|(names, _)| names.clone()),
            Some(vec!["tcp-443".to_string(), "tcp-80".to_string()]),
            "the very first start must already hand over both descriptors, in order"
        );
    }

    /// **The whole running-machine lifecycle**: an install that adds a
    /// `plain:` rule updates the proxy's STORED listen without restarting it
    /// or touching its live descriptors, and warns naming the exact command;
    /// an operator's restart is what makes port 80 live; and removing the
    /// rule and restarting again closes it.
    ///
    /// `ADR-055` §6 leaves whether an install may restart a service nobody
    /// asked it to as the maintainer's open question — this only journals,
    /// it never restarts on its own.
    #[test]
    fn an_install_adding_then_removing_a_plain_rule_only_takes_effect_on_a_restart() {
        let mut k = FakeKernel::new();
        let mut sup = image_proxy_machine(&mut k, true);

        // --- Boot: no plain rule, the proxy starts on 443 alone. -----------
        let installed = app_list(&["rivet-proxyd", "notes-rivet"]);
        let plain = publish_routes(&mut k, sup.journal_mut(), &installed);
        reconcile_proxy_listen(&mut k, &mut sup, plain);
        sup.converge(&mut k);
        let crate::supervise::State::Started { pid: mut running, .. } = sup
            .services()
            .iter()
            .find(|m| m.def.name == rivet_core::service::PROXYD_SERVICE_NAME)
            .map(|m| m.state.clone())
            .expect("the proxy is a service this supervisor knows")
        else {
            panic!("the proxy did not start");
        };
        assert_eq!(
            k.listeners_open
                .get(rivet_core::service::PROXYD_SERVICE_NAME)
                .map(|(names, _)| names.clone()),
            Some(vec!["tcp-443".to_string()])
        );

        // --- A running-machine install adds a plain: rule. ------------------
        install_app(
            &mut k,
            "gantry-rivet",
            &app_manifest_with_rules("gantry", &["plain:install"], vec![Socket::Tcp(8080)]),
        );
        let installed = app_list(&["rivet-proxyd", "notes-rivet", "gantry-rivet"]);
        let plain = publish_routes(&mut k, sup.journal_mut(), &installed);
        assert!(plain);
        reconcile_proxy_listen(&mut k, &mut sup, plain);

        assert_eq!(
            proxy_listen_names(&sup),
            vec!["tcp-443".to_string(), "tcp-80".to_string()],
            "the STORED listen must reflect the new table immediately"
        );
        assert!(
            matches!(
                sup.services()
                    .iter()
                    .find(|m| m.def.name == rivet_core::service::PROXYD_SERVICE_NAME)
                    .map(|m| &m.state),
                Some(crate::supervise::State::Started { .. })
            ),
            "reconciling the stored listen must not itself stop the service"
        );
        assert_eq!(
            k.listeners_open
                .get(rivet_core::service::PROXYD_SERVICE_NAME)
                .map(|(names, _)| names.clone()),
            Some(vec!["tcp-443".to_string()]),
            "the RUNNING proxy's actual descriptors must not change without a restart"
        );
        let said: Vec<String> = sup.journal_mut().pending().iter().map(|r| r.msg.clone()).collect();
        assert!(
            said.iter().any(|m| {
                m.contains("gained port 80") && m.contains("apply service-restart rivet-proxyd confirm")
            }),
            "no warning named the change and the restart command: {said:?}"
        );

        // --- An operator's restart is what makes it live. -------------------
        sup.restart_service(&mut k, rivet_core::service::PROXYD_SERVICE_NAME, true)
            .expect("permitted");
        sup.converge(&mut k); // Started -> Stopping
        k.exits.push((running, crate::kernel::ExitStatus::Signalled(15)));
        sup.reap(&mut k); // Stopping -> Idle
        sup.converge(&mut k); // Idle -> Started, with the new listen
        assert_eq!(
            k.listeners_open
                .get(rivet_core::service::PROXYD_SERVICE_NAME)
                .map(|(names, _)| names.clone()),
            Some(vec!["tcp-443".to_string(), "tcp-80".to_string()]),
            "a restart must bind port 80 once it is newly required"
        );
        let crate::supervise::State::Started { pid: new_running, .. } = sup
            .services()
            .iter()
            .find(|m| m.def.name == rivet_core::service::PROXYD_SERVICE_NAME)
            .map(|m| m.state.clone())
            .expect("the proxy is a service this supervisor knows")
        else {
            panic!("the restart left the proxy down");
        };
        running = new_running;

        // --- The rule is removed; the stored listen drops 80 and warns. -----
        let installed = app_list(&["rivet-proxyd", "notes-rivet"]);
        let plain = publish_routes(&mut k, sup.journal_mut(), &installed);
        assert!(!plain);
        reconcile_proxy_listen(&mut k, &mut sup, plain);
        assert_eq!(proxy_listen_names(&sup), vec!["tcp-443".to_string()]);
        assert_eq!(
            k.listeners_open
                .get(rivet_core::service::PROXYD_SERVICE_NAME)
                .map(|(names, _)| names.clone()),
            Some(vec!["tcp-443".to_string(), "tcp-80".to_string()]),
            "still running with 80 open until the next restart"
        );

        // --- A second restart closes port 80. --------------------------------
        sup.restart_service(&mut k, rivet_core::service::PROXYD_SERVICE_NAME, true)
            .expect("permitted");
        sup.converge(&mut k); // Started -> Stopping
        k.exits.push((running, crate::kernel::ExitStatus::Signalled(15)));
        sup.reap(&mut k); // Stopping -> Idle
        sup.converge(&mut k); // Idle -> Started, on 443 alone again
        assert_eq!(
            k.listeners_open
                .get(rivet_core::service::PROXYD_SERVICE_NAME)
                .map(|(names, _)| names.clone()),
            Some(vec!["tcp-443".to_string()]),
            "the restart must close port 80 once it is no longer required"
        );
        assert!(
            k.listeners_closed
                .iter()
                .filter(|s| s.as_str() == rivet_core::service::PROXYD_SERVICE_NAME)
                .count()
                >= 2,
            "each restart's re-entry must close the previous listeners first: {:?}",
            k.listeners_closed
        );
    }

    // -----------------------------------------------------------------------
    // What the applications offer to back up — ADR-051
    // -----------------------------------------------------------------------

    use rivet_core::package::application::{BackupFormat, BackupSurface};

    /// A manifest asking for a route and offering a backup on it.
    fn backup_manifest(path: Option<&str>, surface: Option<BackupSurface>) -> String {
        Manifest::new(ProgramKind::Daemon)
            .with_listen(vec![Socket::Tcp(8081)])
            .expect("a short list")
            .with_proxy_path(path.map(ToString::to_string))
            .expect("a legal segment")
            .with_backup(surface)
            .expect("a legal backup path")
            .render()
    }

    /// What was written to the machine's backup table, if anything was.
    fn backups(k: &FakeKernel) -> Option<String> {
        k.writes
            .iter()
            .find(|w| w.path == rivet_core::backup::PATH)
            .map(|w| String::from_utf8(w.contents.clone()).expect("utf-8"))
    }

    #[test]
    fn every_declared_surface_reaches_the_table_and_nothing_else_does() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install_app(
            &mut k,
            "tempmon-rivet",
            &backup_manifest(
                Some("tempmon"),
                Some(BackupSurface::Artifact {
                    path: "api/v1/backup".to_string(),
                    format: BackupFormat::Sqlite,
                    restore: Some("api/v1/restore".to_string()),
                }),
            ),
        );
        install_app(
            &mut k,
            "notes-rivet",
            &backup_manifest(
                Some("notes"),
                Some(BackupSurface::Artifact {
                    path: "api/backup".to_string(),
                    format: BackupFormat::Json,
                    restore: None,
                }),
            ),
        );
        install_app(
            &mut k,
            "reliquary-rivet",
            &backup_manifest(
                Some("reliquary"),
                Some(BackupSurface::Feed {
                    path: "api/v1/blobs".to_string(),
                }),
            ),
        );
        // Routed, and offering nothing. `ADR-030`'s exclusion still standing.
        install_app(
            &mut k,
            "aura-controller",
            &backup_manifest(Some("aura"), None),
        );

        publish_backups(
            &mut k,
            &mut j,
            &app_list(&[
                "tempmon-rivet",
                "notes-rivet",
                "reliquary-rivet",
                "aura-controller",
            ]),
        );

        let text = backups(&k).expect("no table was written");
        let declared = rivet_core::backup::parse(&text);
        assert_eq!(declared.len(), 3, "{text}");
        let tempmon = declared
            .iter()
            .find(|d| d.service == "tempmon-rivet")
            .expect("tempmon");
        assert_eq!(tempmon.request_path(), "/tempmon/api/v1/backup");
        assert_eq!(tempmon.surface.format(), Some(BackupFormat::Sqlite));
        // `ADR-051` §12: the restore route rides from the installed manifest
        // into the table a workstation reads, and only where it was declared.
        assert_eq!(
            tempmon.restore_request_path().as_deref(),
            Some("/tempmon/api/v1/restore"),
            "{text}"
        );
        let notes = declared
            .iter()
            .find(|d| d.service == "notes-rivet")
            .expect("notes");
        assert_eq!(notes.restore_request_path(), None, "{text}");
        let reliquary = declared
            .iter()
            .find(|d| d.service == "reliquary-rivet")
            .expect("reliquary");
        assert_eq!(reliquary.request_path(), "/reliquary/api/v1/blobs");
        assert_eq!(reliquary.surface.format(), None);
        assert!(
            !declared.iter().any(|d| d.service == "aura-controller"),
            "an application that offers nothing must not appear: {text}"
        );
    }

    /// **An empty table is written rather than omitted**, so that an operator
    /// asking a machine what it offers can tell *nothing* from *this build does
    /// not answer that question*.
    #[test]
    fn a_machine_where_nothing_offers_a_backup_still_gets_an_empty_table() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install_app(
            &mut k,
            "aura-controller",
            &backup_manifest(Some("aura"), None),
        );
        publish_backups(&mut k, &mut j, &app_list(&["aura-controller"]));
        assert_eq!(backups(&k).as_deref(), Some(""));
    }

    /// **Nothing in the table comes from the document, and nothing here is
    /// fatal.** A manifest that will not parse is skipped and the rest are
    /// still published — a machine must not stop converging over a backup
    /// surface, because a machine that stops converging is one nothing can fix
    /// remotely.
    #[test]
    fn an_unreadable_manifest_is_skipped_and_the_rest_are_still_published() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install_app(&mut k, "broken-rivet", "this is not a manifest\n");
        install_app(
            &mut k,
            "tempmon-rivet",
            &backup_manifest(
                Some("tempmon"),
                Some(BackupSurface::Artifact {
                    path: "api/v1/backup".to_string(),
                    format: BackupFormat::Sqlite,
                    restore: None,
                }),
            ),
        );
        publish_backups(
            &mut k,
            &mut j,
            &app_list(&["broken-rivet", "tempmon-rivet"]),
        );
        let text = backups(&k).expect("no table was written");
        assert_eq!(rivet_core::backup::parse(&text).len(), 1, "{text}");
    }

    /// The table is a function of the set of installed applications rather than
    /// of the order the supervisor happened to read them in.
    #[test]
    fn the_table_does_not_depend_on_the_order_applications_were_discovered() {
        let surface = |path: &str| {
            Some(BackupSurface::Artifact {
                path: path.to_string(),
                format: BackupFormat::Csv,
                restore: None,
            })
        };
        let render_for = |names: &[&str]| {
            let mut k = FakeKernel::new();
            let mut j = journal();
            install_app(&mut k, "a-rivet", &backup_manifest(Some("a"), surface("x/y")));
            install_app(&mut k, "b-rivet", &backup_manifest(Some("b"), surface("x/y")));
            publish_backups(&mut k, &mut j, &app_list(names));
            backups(&k).expect("a table")
        };
        assert_eq!(
            render_for(&["a-rivet", "b-rivet"]),
            render_for(&["b-rivet", "a-rivet"])
        );
    }

    // -----------------------------------------------------------------------
    // The firewall — ADR-023 §3 and §9
    // -----------------------------------------------------------------------

    fn proxy_uid() -> u32 {
        rivet_core::service::app_identity(
            &rivet_core::service::ServiceName::new(PROXY_SERVICE).expect("a name"),
        )
        .uid
    }

    /// **Only the routed port of a proxied application is guarded**, and the
    /// proxy and root are the two uids admitted to it. A second port of the
    /// same application, a `:direct` one, and every port of an application
    /// with no `proxy-path` stay the application's own.
    #[test]
    fn the_policy_guards_each_routed_port_and_nothing_else() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install_app(&mut k, "rivet-proxyd", &app_manifest(None, ProxyAuth::Operator, vec![Socket::Tcp(443)]));
        install_app(
            &mut k,
            "notes-rivet",
            &app_manifest(Some("notes"), ProxyAuth::Operator, vec![Socket::Tcp(8082), Socket::Tcp(9100)]),
        );
        install_app(&mut k, "plain-rivet", &app_manifest(None, ProxyAuth::Operator, vec![Socket::Tcp(8500)]));
        let policy = firewall_policy(
            &mut k,
            &mut j,
            &app_list(&["rivet-proxyd", "notes-rivet", "plain-rivet"]),
        );
        assert_eq!(policy.ports(), &[8082]);
        assert_eq!(policy.admitted(), &[0, proxy_uid()]);
    }

    /// No proxy installed: no caller to admit, so nothing is guarded.
    #[test]
    fn a_machine_without_the_proxy_guards_nothing() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install_app(&mut k, "notes-rivet", &app_manifest(Some("notes"), ProxyAuth::Operator, vec![Socket::Tcp(8082)]));
        let policy = firewall_policy(&mut k, &mut j, &app_list(&["notes-rivet"]));
        assert!(policy.ports().is_empty());
    }

    /// **The control plane's port never enters the policy**, whatever a
    /// manifest says, and the refusal is journalled by name.
    #[test]
    fn a_routed_port_on_the_control_planes_is_never_guarded() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install_app(&mut k, "rivet-proxyd", &app_manifest(None, ProxyAuth::Operator, vec![Socket::Tcp(443)]));
        install_app(
            &mut k,
            "odd-rivet",
            &app_manifest(
                Some("odd"),
                ProxyAuth::Operator,
                vec![Socket::Tcp(rivet_config::control_plane::DEFAULT_PORT)],
            ),
        );
        let policy = firewall_policy(&mut k, &mut j, &app_list(&["rivet-proxyd", "odd-rivet"]));
        assert!(policy.ports().is_empty(), "{policy:?}");
        let said: Vec<String> = j.pending().iter().map(|r| r.msg.clone()).collect();
        assert!(said.iter().any(|m| m.contains("control plane")), "{said:?}");
    }

    /// **Applied once per change**: the boot applies, an event that changes
    /// nothing does not touch the kernel again, and the journal says what the
    /// kernel read back.
    #[test]
    fn the_firewall_is_applied_once_per_change_and_reported_as_read_back() {
        let mut k = FakeKernel::new();
        let mut sup = image_proxy_machine(&mut k, true);
        reconcile_firewall(&mut k, &mut sup);
        reconcile_firewall(&mut k, &mut sup);
        assert_eq!(k.firewalls.len(), 1, "an unchanged policy was applied twice");
        assert_eq!(k.firewalls[0].ports(), &[8082]);
        let said: Vec<String> = sup.journal_mut().pending().iter().map(|r| r.msg.clone()).collect();
        assert!(said.iter().any(|m| m.contains("in place and was read back")), "{said:?}");
    }

    /// **A refusal is journalled and stops nothing.** The kernel applies a
    /// refused transaction not at all, and the record says networking is as
    /// it was.
    #[test]
    fn a_refused_firewall_is_reported_and_is_not_fatal() {
        let mut k = FakeKernel::new();
        k.firewall_result = Some(Err("create chain proxied-backends: the kernel refused".to_string()));
        let mut sup = image_proxy_machine(&mut k, true);
        reconcile_firewall(&mut k, &mut sup);
        let said: Vec<String> = sup.journal_mut().pending().iter().map(|r| r.msg.clone()).collect();
        assert!(said.iter().any(|m| m.contains("could NOT be applied")), "{said:?}");
    }

    /// **A table that reads back differently is not reported as in place.**
    #[test]
    fn a_firewall_that_reads_back_wrong_is_a_warning_not_a_success() {
        let mut k = FakeKernel::new();
        k.firewall_result = Some(Ok(rivet_adapter_net::firewall::Applied {
            found: Vec::new(),
            unrecognised: 1,
            raw: Vec::new(),
        }));
        let mut sup = image_proxy_machine(&mut k, true);
        reconcile_firewall(&mut k, &mut sup);
        let said: Vec<String> = sup.journal_mut().pending().iter().map(|r| r.msg.clone()).collect();
        assert!(said.iter().any(|m| m.contains("does NOT read back")), "{said:?}");
        assert!(!said.iter().any(|m| m.contains("in place and was read back")), "{said:?}");
    }

    // -----------------------------------------------------------------------
    // An uninstall rewrites the routing table — ADR-046 §5
    // -----------------------------------------------------------------------

    /// **A removed application's route leaves the table at the uninstall**,
    /// not at the next install or boot. The table is the last thing written to
    /// the proxy's home, so a route left in it is a route to an application
    /// that is gone, served from the proxy's next start onward.
    #[test]
    fn an_uninstall_rewrites_the_routing_table_without_the_removed_route() {
        let mut k = FakeKernel::new();
        let mut sup = image_proxy_machine(&mut k, true);
        let installed = installed_applications(sup.services());
        let plain = publish_routes(&mut k, sup.journal_mut(), &installed);
        reconcile_proxy_listen(&mut k, &mut sup, plain);
        let before = table(&k).expect("the boot wrote a table");
        assert_eq!(rivet_core::proxy::parse(&before).len(), 1, "{before}");

        sup.begin_uninstall(&mut k, "notes-rivet", true);

        let want = rivet_core::proxy::path(
            rivet_core::service::BUILT_IN_VOLUME,
            &rivet_core::service::ServiceName::new(PROXY_SERVICE).expect("a name"),
        );
        let writes = k.writes.iter().filter(|w| w.path == want).count();
        assert_eq!(writes, 2, "the uninstall did not rewrite the routing table");
        let after = k
            .writes
            .iter()
            .rev()
            .find(|w| w.path == want)
            .map(|w| String::from_utf8(w.contents.clone()).expect("utf-8"))
            .expect("a table");
        assert!(
            rivet_core::proxy::parse(&after).is_empty(),
            "the removed application is still routed: {after}"
        );
    }

    /// **A removed application's backup surface leaves the table at the
    /// uninstall** — `ADR-051` — not at the next install or boot. A backup
    /// client reading the table would otherwise ask an application that is
    /// gone for its data.
    #[test]
    fn an_uninstall_rewrites_the_backup_table_without_the_removed_surface() {
        let mut k = FakeKernel::new();
        let manifest = backup_manifest(
            Some("notes"),
            Some(BackupSurface::Artifact {
                path: "api/v1/backup".to_string(),
                format: BackupFormat::Sqlite,
                restore: None,
            }),
        );
        install_app(&mut k, "notes-rivet", &manifest);
        let mut defs = crate::definitions::system_services();
        defs.push(crate::applications::definition(
            rivet_core::service::BUILT_IN_VOLUME,
            &rivet_core::service::ServiceName::new("notes-rivet").expect("a name"),
            &Manifest::parse(manifest.as_bytes()).expect("a manifest"),
            &[],
        ));
        let mut sup =
            match crate::supervise::Supervisor::new(defs, crate::service::Limits::default(), journal()) {
                Ok(s) => s,
                Err(e) => panic!("fixture refused: {e}"),
            };
        let installed = installed_applications(sup.services());
        publish_backups(&mut k, sup.journal_mut(), &installed);
        let before = backups(&k).expect("the boot wrote a table");
        assert_eq!(rivet_core::backup::parse(&before).len(), 1, "{before}");

        sup.begin_uninstall(&mut k, "notes-rivet", true);

        let writes: Vec<&_> = k
            .writes
            .iter()
            .filter(|w| w.path == rivet_core::backup::PATH)
            .collect();
        assert_eq!(writes.len(), 2, "the uninstall did not rewrite the backup table");
        let after = writes
            .last()
            .map(|w| String::from_utf8(w.contents.clone()).expect("utf-8"))
            .expect("a table");
        assert!(
            rivet_core::backup::parse(&after).is_empty(),
            "the removed application still offers a backup: {after}"
        );
    }
}
