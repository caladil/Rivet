//! **Whether an application's declared per-machine material is on this
//! machine yet** — `ADR-055` §3, checked immediately before every launch.
//!
//! Three applications on a real board declared `material = ["fleet-tls"]`,
//! were enabled before an operator had installed their TLS files, and were
//! spawned anyway — several times a second, forever, each crash landing in
//! the middle of loading its add-ons and leaving trial notes behind that then
//! made it refuse its add-ons on every later start too. `crate::verify`
//! already refuses to spawn a program whose signature will not check out;
//! this is the identical idea applied to a precondition the manifest itself
//! names, rather than to the bytes on disk.
//!
//! # What this checks, and what it does not
//!
//! [`missing`] asks the machine, not the manifest's own memory of what it was
//! given — a `stat` on each file `Material::files` names, in the application's
//! `/state/app/<name>/data`, every time it is called. It mints nothing and
//! writes nothing: `ADR-055` §6 keeps minting and installing an operator's
//! act, permanently, so all this module can do is say what is missing and
//! what to run.
//!
//! `crate::supervise::Supervisor::start` is the sole caller. It runs the check
//! after the adoption read (an already-running process is left alone
//! regardless of what its data directory holds today) and before
//! `crate::verify::program`, and holds the service at
//! [`crate::supervise::State::WaitingForMaterial`] rather than spawning it —
//! see that state's own documentation for why it is a distinct state and not
//! `Failed` reached by a different route, unlike `ADR-017` §3's precedent for
//! a program the image cannot run.

use crate::kernel::Kernel;
use crate::service::ServiceDef;

/// One file a manifest declared that is not, right now, in the application's
/// data directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingFile {
    /// The kind's own word — `fleet-tls`, today the only one `ADR-055` §3
    /// defines.
    pub kind: &'static str,
    /// The file's name under `.../data`, from [`rivet_core::package::
    /// application::Material::files`].
    pub file: &'static str,
}

impl core::fmt::Display for MissingFile {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} ({})", self.file, self.kind)
    }
}

/// Which of `def`'s declared material is missing from its data directory.
///
/// **Empty for a service that declares none**, which is every image service
/// and every application whose manifest carries no `material` key —
/// `ServiceDef::material` is empty for both, so this returns before asking the
/// kernel anything. Empty is also the answer for a definition with no volume
/// or an unusable name, which `crate::applications::definition` never
/// produces; nothing but a hand-built fixture reaches that branch, and it is
/// refused the same as "nothing declared" rather than treated as a machine
/// this cannot be asked about.
///
/// # `Err` from the kernel is treated as missing, not skipped
///
/// `AGENTS.md` §3: refuse rather than guess. `Kernel::file_exists` answering
/// "I could not tell" is not "the file is there" — spawning on the strength of
/// a read that failed is the same silent wrong answer the crash-loop this
/// exists to end was already producing, from a different cause.
pub fn missing(k: &mut dyn Kernel, def: &ServiceDef) -> Vec<MissingFile> {
    if def.material.is_empty() {
        return Vec::new();
    }
    let Some(volume) = def.volume.as_deref() else {
        return Vec::new();
    };
    let Some(name) = rivet_core::service::ServiceName::new(&def.name) else {
        return Vec::new();
    };
    let data_dir = rivet_core::service::data_dir(volume, &name);

    let mut out = Vec::new();
    for kind in &def.material {
        for file in kind.files() {
            let path = format!("{data_dir}/{file}");
            match k.file_exists(&path) {
                Ok(true) => {}
                Ok(false) | Err(_) => out.push(MissingFile {
                    kind: kind.as_str(),
                    file,
                }),
            }
        }
    }
    out
}

/// **The one sentence** a held service's journal record and its `get status`
/// `reason` both carry — `AGENTS.md` §5, one fact, one wording — naming the
/// application, what is missing, and the fix in words.
///
/// `issued_here` is whether this machine holds its fleet's root and so issues
/// the material itself (`crate::issuance`). There the fix is not a command: the
/// material is issued at install and at every boot, and an application still
/// held is one whose issuance failed, which the journal says. Elsewhere minting
/// and installing stay an operator's act (`ADR-055` §6), so this says what to
/// run.
pub fn describe(app: &str, missing: &[MissingFile], issued_here: bool) -> String {
    let list = missing
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    if issued_here {
        return format!(
            "HELD waiting-for-material: this application declares per-machine material (ADR-055 \
             §3) and its data directory is missing {list}. This machine holds its fleet's root \
             and issues that material itself, at install and at every boot (ADR-063 §4), so \
             the journal's record of the last issuance says why it is not there; this \
             supervisor starts it on its own next check once it is"
        );
    }
    format!(
        "HELD waiting-for-material: this application declares per-machine material (ADR-055 §3) \
         and its data directory is missing {list}. Install it from a workstation holding the \
         fleet authority with `cargo rivet secrets {app} --host HOST:PORT --identity DIR --pin \
         HEX confirm`, or file by file with `rivet apply install-secret {app} <name> --hex HEX \
         confirm`; this supervisor starts it on its own next check, with no reload or restart \
         needed"
    )
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::service::{IMAGE_DEFINED, Origin};
    use crate::testkernel::{FakeKernel, Script};
    use rivet_core::package::application::Material;

    fn app_def(name: &str, material: Vec<Material>) -> ServiceDef {
        let service_name = rivet_core::service::ServiceName::new(name)
            .unwrap_or_else(|| unreachable!("{name:?} is not a service name"));
        ServiceDef {
            name: name.to_string(),
            origin: Origin::Application,
            volume: Some(rivet_core::service::BUILT_IN_VOLUME.to_string()),
            material,
            program: crate::kernel::Program {
                path: String::new(),
                args: Vec::new(),
                env: Vec::new(),
                identity: Some(rivet_core::service::app_identity(&service_name)),
                confinement: None,
                groups: Vec::new(),
                capabilities: Vec::new(),
            },
            ..IMAGE_DEFINED
        }
    }

    #[test]
    fn a_service_that_declares_no_material_is_never_asked_about() {
        let mut k = FakeKernel::new();
        let def = app_def("a", Vec::new());
        assert_eq!(missing(&mut k, &def), Vec::new());
        assert!(
            k.exists_asked.is_empty(),
            "a service with nothing declared must not be asked about any path: {:?}",
            k.exists_asked
        );
    }

    #[test]
    fn every_declared_file_absent_is_named() {
        let mut k = FakeKernel::new();
        let def = app_def("a", vec![Material::FleetTls]);
        let found = missing(&mut k, &def);
        assert_eq!(found.len(), 3, "{found:?}");
        for file in ["tls-cert", "tls-key", "tls-root"] {
            assert!(
                found.iter().any(|m| m.file == file),
                "{file} was not reported missing: {found:?}"
            );
        }
    }

    #[test]
    fn files_that_are_present_are_not_reported() {
        let mut k = FakeKernel::new();
        let def = app_def("a", vec![Material::FleetTls]);
        let data = rivet_core::service::data_dir(
            rivet_core::service::BUILT_IN_VOLUME,
            &rivet_core::service::ServiceName::new("a").expect("a valid fixture name"),
        );
        for file in ["tls-cert", "tls-key", "tls-root"] {
            k.exists_mut()
                .insert(format!("{data}/{file}"), Script::always(Ok(true)));
        }
        assert_eq!(missing(&mut k, &def), Vec::new());
    }

    /// A read the kernel could not answer is treated as missing, never as
    /// present — `AGENTS.md` §3.
    #[test]
    fn a_read_that_could_not_be_answered_is_treated_as_missing() {
        let mut k = FakeKernel::new();
        let def = app_def("a", vec![Material::FleetTls]);
        let data = rivet_core::service::data_dir(
            rivet_core::service::BUILT_IN_VOLUME,
            &rivet_core::service::ServiceName::new("a").expect("a valid fixture name"),
        );
        k.exists_mut().insert(
            format!("{data}/tls-cert"),
            Script::always(Err(crate::kernel::Errno::EIO)),
        );
        k.exists_mut()
            .insert(format!("{data}/tls-key"), Script::always(Ok(true)));
        k.exists_mut()
            .insert(format!("{data}/tls-root"), Script::always(Ok(true)));
        let found = missing(&mut k, &def);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].file, "tls-cert");
    }

    #[test]
    fn the_description_names_the_application_the_files_and_both_commands() {
        let text = describe(
            "aura-controller",
            &[
                MissingFile {
                    kind: "fleet-tls",
                    file: "tls-cert",
                },
                MissingFile {
                    kind: "fleet-tls",
                    file: "tls-key",
                },
            ],
            false,
        );
        assert!(text.contains("aura-controller"), "{text}");
        assert!(text.contains("tls-cert (fleet-tls)"), "{text}");
        assert!(text.contains("tls-key (fleet-tls)"), "{text}");
        assert!(text.contains("cargo rivet secrets aura-controller"), "{text}");
        assert!(
            text.contains("rivet apply install-secret aura-controller"),
            "{text}"
        );
        assert!(!text.contains("  "), "{text}");
    }

    /// **On a machine that issues its own material, no command is offered**:
    /// there is nothing for an operator to run, and the workstation's
    /// authority could not sign for this fleet's root anyway.
    #[test]
    fn on_a_fleet_root_machine_the_description_offers_no_command() {
        let text = describe(
            "aura-controller",
            &[MissingFile {
                kind: "fleet-tls",
                file: "tls-cert",
            }],
            true,
        );
        assert!(text.contains("tls-cert (fleet-tls)"), "{text}");
        assert!(text.contains("issues that material itself"), "{text}");
        assert!(!text.contains("cargo rivet secrets"), "{text}");
        assert!(!text.contains("install-secret"), "{text}");
        assert!(!text.contains("  "), "{text}");
    }
}
