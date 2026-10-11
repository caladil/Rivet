//! **A fleet-root machine issues its applications' certificates itself** —
//! `ADR-063` §4.
//!
//! On a machine whose `/state/secrets` holds its fleet's root, every installed
//! application that declares `fleet-tls` is given a leaf of its own, signed by
//! that root, carrying the machine's own names, with a key generated here. The
//! proxy's and the fleet service's shared leaf is re-issued the same way. A
//! machine holding no root issues nothing: its material comes from a
//! workstation's authority, as it always has.
//!
//! # When
//!
//! [`converge`] runs at every boot, before the first service is started, and
//! after every install, before the newly installed application is adopted.
//! Each time it judges every `fleet-tls` application's leaf, and at boot the
//! shared leaf too, and issues where one is missing, does not chain to this
//! root, ends within ninety days, or lacks the standard fields
//! (`rivet_fleet_root::judge`).
//!
//! # Who holds the root's key
//!
//! **Nothing here reads it.** Judging needs only the root's certificate, which
//! is public. Issuing is [`Kernel::issue_in_child`]: a helper forked per
//! issuance reads the key after the fork, signs, writes and exits, so the key
//! is never in the supervisor's memory. A fork is enough for that, and no
//! separate program is needed: `ADR-063` keeps the root's key out of any
//! process that serves a network, and the helper serves none and lives for
//! one signature, while the supervisor's own address space never receives a
//! byte of the key.
//!
//! # Restarting what reads a new leaf
//!
//! At boot nothing has started yet, so every program reads its new leaf at its
//! first start — the proxy included — and nothing is restarted. After an
//! install, an application that is already running keeps the leaf it was
//! started with until its next start; that is journalled, and nothing is
//! restarted, for the reason `crate::installed` gives.

use rivet_core::fleet_root::{
    HANDED_TO, MINTED_FOR_FILE, SERVER_CERTIFICATE_FILE, SERVER_KEY_FILE, root_certificate_path,
    root_key_path,
};
use rivet_core::package::application::Material;
use rivet_core::service::{self, ServiceName};
use rivet_fleet_root::judge::{Root, Verdict};

use crate::journal::{Journal, Severity, Value};
use crate::kernel::{Errno, IssueOrder, IssueTarget, Kernel};
use crate::service::{Origin, ServiceDef};
use crate::supervise::SELF_NAME;

/// The most this reads of any certificate or request.
const MAX_PEM_BYTES: usize = 64 * 1024;

/// What prompted a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Occasion {
    /// A boot, before anything is started: every leaf is judged.
    Boot,
    /// An install has finished: every `fleet-tls` application's leaf is
    /// judged; the shared leaf is left for the next boot, because the proxy is
    /// running and reads its leaf only at its start.
    Install,
}

/// Whether this machine holds its fleet's root, and so issues its own
/// applications' certificates. A root that cannot be asked about is not held.
pub fn issues_here(k: &mut dyn Kernel) -> bool {
    k.file_exists(&root_certificate_path()) == Ok(true)
}

/// **Judge every leaf this machine issues, and issue where one is wanted.**
/// `running` names the services running now, for the journal.
pub fn converge(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    defs: &[ServiceDef],
    occasion: Occasion,
    running: &[String],
) {
    match k.file_exists(&root_certificate_path()) {
        Ok(true) => {}
        // A machine whose fleet's root is on a workstation. Nothing to do,
        // and nothing to say on every boot of every such machine.
        Ok(false) => return,
        Err(e) => {
            note(
                k,
                journal,
                Severity::Error,
                SELF_NAME,
                "DEGRADED whether this machine holds its fleet's root could not be read, so no \
                 certificate was judged or issued this time",
                vec![
                    ("path".to_string(), Value::Str(root_certificate_path())),
                    ("detail".to_string(), Value::Str(e.to_string())),
                ],
            );
            return;
        }
    }
    let Some((root, root_pem, names)) = read_root(k, journal) else {
        return;
    };
    let now = k.wall_ms().map_or(0, |ms| ms / 1000);
    let base = Base {
        root: &root,
        root_pem: &root_pem,
        names: &names,
        now,
    };

    if occasion == Occasion::Boot {
        server_leaf(k, journal, defs, &base);
    }
    for def in defs {
        if def.origin == Origin::Application && def.material.contains(&Material::FleetTls) {
            application_leaf(k, journal, def, &base, running.contains(&def.name));
        }
    }
}

/// What every judgement and order is made from.
struct Base<'a> {
    root: &'a Root,
    root_pem: &'a str,
    names: &'a [String],
    now: u64,
}

impl Base<'_> {
    fn order(&self, targets: Vec<IssueTarget>) -> IssueOrder {
        IssueOrder {
            root_certificate: root_certificate_path(),
            root_key: root_key_path(),
            names: self.names.to_vec(),
            now: self.now,
            targets,
        }
    }
}

/// The root's certificate and the names the machine's leaves carry, read from
/// `/state/secrets` — both public.
fn read_root(k: &mut dyn Kernel, journal: &mut Journal) -> Option<(Root, String, Vec<String>)> {
    let request_path = format!("{}/{MINTED_FOR_FILE}", rivet_core::state::SECRETS);
    let read = |k: &mut dyn Kernel, path: &str| -> Result<String, String> {
        let bytes = k
            .read_file_bytes(path, MAX_PEM_BYTES)
            .map_err(|e| format!("{path} could not be read: {e}"))?;
        String::from_utf8(bytes).map_err(|_| format!("{path} is not text"))
    };
    let found = read(k, &root_certificate_path()).and_then(|pem| {
        let root = Root::from_pem(&pem)?;
        let request = read(k, &request_path)?;
        let request = rivet_core::fleet_root::parse(&request)
            .map_err(|why| format!("{request_path} will not read: {why}"))?;
        Ok((root, pem, request.names()))
    });
    match found {
        Ok(found) => Some(found),
        Err(detail) => {
            note(
                k,
                journal,
                Severity::Error,
                SELF_NAME,
                "DEGRADED this machine holds its fleet's root and it could not be read, so no \
                 certificate was judged or issued this time. Applications whose certificate is \
                 missing stay held until one is",
                vec![("detail".to_string(), Value::Str(detail))],
            );
            None
        }
    }
}

/// What one leaf on the disk is, and why it wants replacing when it does.
fn judged(k: &mut dyn Kernel, path: &str, base: &Base<'_>) -> Verdict {
    match k.read_file_bytes(path, MAX_PEM_BYTES) {
        Ok(bytes) => {
            rivet_fleet_root::judge::judge(&String::from_utf8_lossy(&bytes), base.root, base.now)
        }
        Err(Errno::ENOENT) => Verdict::Missing("the certificate".to_string()),
        Err(e) => Verdict::Unreadable(format!("it could not be read: {e}")),
    }
}

/// An application's data directory, and who owns it.
fn home(def: &ServiceDef) -> Option<(String, u32, u32)> {
    let id = def.program.identity?;
    let volume = def.volume.as_deref()?;
    let name = ServiceName::new(&def.name)?;
    Some((service::data_dir(volume, &name), id.uid, id.gid))
}

/// **The proxy's and the fleet service's leaf** — one leaf, under each
/// program's own names, and resting in `/state/secrets` beside the root, as
/// the first boot hands it over (`rivet_core::fleet_root::HANDED_TO`).
fn server_leaf(k: &mut dyn Kernel, journal: &mut Journal, defs: &[ServiceDef], base: &Base<'_>) {
    let mut targets = vec![IssueTarget {
        dir: rivet_core::state::SECRETS.to_string(),
        certificate: SERVER_CERTIFICATE_FILE.to_string(),
        key: SERVER_KEY_FILE.to_string(),
        root: None,
        uid: 0,
        gid: 0,
    }];
    let mut reasons: Vec<(String, Verdict)> = Vec::new();
    for handed in HANDED_TO {
        let Some(def) = defs.iter().find(|d| d.name == handed.application) else {
            continue;
        };
        let Some((dir, uid, gid)) = home(def) else {
            continue;
        };
        // A program the document leaves off has no home, and a leaf cannot be
        // written into a directory that is not there.
        if k.file_exists(&dir) != Ok(true) {
            continue;
        }
        let verdict = judged(k, &format!("{dir}/{}", handed.certificate), base);
        if verdict.wants_issuance() {
            reasons.push((handed.application.to_string(), verdict));
        }
        targets.push(IssueTarget {
            dir,
            certificate: handed.certificate.to_string(),
            key: handed.key.to_string(),
            root: Some(handed.root.to_string()),
            uid,
            gid,
        });
    }
    if reasons.is_empty() {
        return;
    }
    let holders: Vec<String> = targets.iter().skip(1).map(|t| t.dir.clone()).collect();
    let why = reasons
        .iter()
        .map(|(app, verdict)| format!("{app}: {}", verdict.reason()))
        .collect::<Vec<_>>()
        .join("; ");
    let order = base.order(targets);
    issue(
        k,
        journal,
        SELF_NAME,
        &order,
        &reasons.iter().map(|(_, v)| v.clone()).collect::<Vec<_>>(),
        &why,
        &holders.join(", "),
        false,
    );
}

/// **One `fleet-tls` application's leaf**, written into its own data
/// directory under `tls-cert`, `tls-key` and `tls-root`.
fn application_leaf(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    def: &ServiceDef,
    base: &Base<'_>,
    running: bool,
) {
    let Some((dir, uid, gid)) = home(def) else {
        return;
    };
    let &[certificate, key, root] = Material::FleetTls.files() else {
        return;
    };
    let mut verdict = judged(k, &format!("{dir}/{certificate}"), base);
    if !verdict.wants_issuance() {
        if k.file_exists(&format!("{dir}/{key}")) != Ok(true) {
            verdict = Verdict::Missing("the certificate's key".to_string());
        } else {
            let held = k
                .read_file_bytes(&format!("{dir}/{root}"), MAX_PEM_BYTES)
                .ok()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
            if held.as_deref() != Some(base.root_pem) {
                verdict = Verdict::Missing("this machine's fleet root as its tls-root".to_string());
            }
        }
    }
    if !verdict.wants_issuance() {
        return;
    }
    let why = verdict.reason();
    let order = base.order(vec![IssueTarget {
        dir: dir.clone(),
        certificate: certificate.to_string(),
        key: key.to_string(),
        root: Some(root.to_string()),
        uid,
        gid,
    }]);
    issue(k, journal, &def.name, &order, &[verdict], &why, &dir, running);
}

/// Fork the helper for `order`, and journal what it did, in `svc`'s name.
#[allow(clippy::too_many_arguments)]
fn issue(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    svc: &str,
    order: &IssueOrder,
    verdicts: &[Verdict],
    why: &str,
    holders: &str,
    running: bool,
) {
    let mut fields = vec![
        ("names".to_string(), Value::Str(order.names.join(", "))),
        ("reason".to_string(), Value::Str(why.to_string())),
        ("path".to_string(), Value::Str(holders.to_string())),
    ];
    let old: Vec<&str> = verdicts
        .iter()
        .filter_map(|v| match v {
            Verdict::Foreign { issuer } => Some(issuer.as_str()),
            _ => None,
        })
        .collect();
    if !old.is_empty() {
        fields.push(("old_issuer".to_string(), Value::Str(old.join(", "))));
    }
    match k.issue_in_child(order) {
        Ok(Ok(summary)) => {
            fields.push(("detail".to_string(), Value::Str(summary)));
            note(
                k,
                journal,
                Severity::Info,
                svc,
                if running {
                    "this machine issued a new certificate from its own fleet root, because the \
                     one held was not good enough; the program is running and reads it at its \
                     next start, so nothing was restarted"
                } else {
                    "this machine issued a new certificate from its own fleet root, because the \
                     one held was not good enough; it is read at the next start"
                },
                fields,
            );
        }
        Ok(Err(detail)) => {
            fields.push(("detail".to_string(), Value::Str(detail)));
            note(
                k,
                journal,
                Severity::Error,
                svc,
                "DEGRADED this machine could not issue a certificate from its own fleet root. \
                 Whatever was held before is still held; an application with none stays held \
                 until one is issued, which is tried again at the next boot or install",
                fields,
            );
        }
        Err(e) => {
            fields.push(("detail".to_string(), Value::Str(e.to_string())));
            note(
                k,
                journal,
                Severity::Error,
                svc,
                "DEGRADED the helper that issues certificates from this machine's fleet root \
                 could not be started, so nothing was issued; it is tried again at the next boot \
                 or install",
                fields,
            );
        }
    }
}

fn note(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    sev: Severity,
    svc: &str,
    msg: &str,
    fields: Vec<(String, Value)>,
) {
    let corr = journal.next_correlation();
    journal.emit(k, sev, svc, &corr, msg, fields);
}

#[cfg(test)]
#[path = "issuance_tests.rs"]
mod tests;
