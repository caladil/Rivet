//! **The third thing PID 1 carries off the boot partition** — `ADR-050` §4,
//! and it is [`crate::staging`]'s journey with the destruction taken **out**.
//!
//! # Why there is a third courier
//!
//! [`crate::identity`] moves the machine's own credential and
//! [`crate::staging`] moves the material an application will be handed. Neither
//! moves an application. `SPEC-05` §1 is the maintainer's — a device that has
//! booted is expected to have `fleetd` up and running just as `mcpd` is
//! (.rulings/ R-303) — and `ADR-049` §5 states the gap in as many words:
//! **none of the applications a fleet server runs is on the card.**
//!
//! `ADR-050` §4 closes it by carrying signed `.rvt` payloads. This module is
//! the hop that gets them off the card; `cmd/rivet-sysd`'s `carried` module is
//! the hop that installs them.
//!
//! # What it does NOT do, and the boundary is the whole design
//!
//! **It does not verify a signature and it does not install anything.**
//!
//! Verification is `rivet_core::package::read::open`'s and happens twice, in
//! neither case here: on the **workstation**, by `cmd/rivet-image`, against the
//! trust store this very card carries — `ADR-050` §4's *"refused where a person
//! can see it"* — and on the machine, by `cmd/rivet-deploy`, which is
//! `ADR-029` §11's single verifier. Putting a third copy in PID 1 would be a
//! package parser in the process whose failure is the machine, for an answer
//! two other programs already give.
//!
//! So this copies bytes. A payload that is corrupt on the card is copied
//! faithfully to `/state` and refused by the installer, in the journal, on a
//! machine that is up and reachable — which is `ADR-050` §10's ruling working
//! rather than a gap in this module.
//!
//! # Nothing is destroyed, and that is the difference from `ADR-049`
//!
//! [`crate::staging`] overwrites the bundle with zeros and unlinks it, because
//! it holds `server.key` and an operator seed and `ADR-020` §11 makes the card
//! the whole boundary. **None of that applies to a `.rvt`**: it is a signed,
//! public artifact whose integrity comes from a signature rather than from
//! secrecy, and the card keeping it means a card in a reader still says what it
//! was built to carry.
//!
//! What replaces the destruction as the thing that stops a payload being
//! installed on every boot for ever is the **record directory**:
//! `rivet_core::carried::record_dir` is created here and is never removed by
//! anybody. A payload whose record directory already exists is skipped, so the
//! second boot of a card copies nothing — and the supervisor removes only the
//! payload *inside* it when the install finishes, so the thing that stops the
//! re-copy survives the install. `rivet_core::carried` carries the argument.
//!
//! # An existing payload is never overwritten
//!
//! [`crate::staging`]'s rule, for its reason: `/state` wins, because a payload
//! already waiting was put there by an earlier boot of this card and may
//! already be halfway through an install. How far it got is the supervisor's to
//! know.

use crate::kernel::Kernel;
use crate::provision::{BOOT_PARTITION_MOUNT, mount_boot_partition, umount_boot_partition};

use rivet_core::carried;

/// Where a carried payload comes to rest.
pub const CARRIED_DIR: &str = carried::CARRIED_DIR;

/// What happened, in the words the console gets.
#[derive(Debug, PartialEq, Eq)]
pub enum Carried {
    /// No payload on the boot partition. **The ordinary case** for every
    /// headless card and for every card built before 2026-09-07.
    Nothing,
    /// Payloads are waiting under [`CARRIED_DIR`]. `written` is how many were
    /// copied on this boot and `kept` how many were already dealt with — either
    /// waiting from an earlier boot or installed and gone.
    Landed { written: usize, kept: usize },
    /// Something was there and could not be completed. **Nothing on the card is
    /// touched**, so the next boot tries again.
    Refused(String),
}

impl Carried {
    /// One line for the console.
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Self::Nothing => format!(
                "no {}*{} on the boot partition; this card carries no application",
                carried::PREFIX,
                carried::SUFFIX
            ),
            Self::Landed { written, kept } => format!(
                "{written} application payload(s) copied to {CARRIED_DIR} and {kept} already \
                 accounted for. NOTHING IS INSTALLED YET and nothing here verified a signature: \
                 the SUPERVISOR asks rivet-deploy to install each one on a convergence pass, and \
                 that is where a bad payload is refused (ADR-050 §4, §10). The card keeps its \
                 copies -- a .rvt is signed, not secret"
            ),
            Self::Refused(why) => format!("no application was carried: {why}"),
        }
    }
}

/// Copy carried payloads off the boot partition into `/state`.
pub fn provision_carried(k: &mut dyn Kernel) -> Carried {
    k.say("rivet-initd: looking for carried application payloads on the boot partition");

    // **Read-only**, unlike [`crate::staging`]'s mount, and the difference is
    // the whole of this module's departure from `ADR-049`: nothing here deletes
    // anything from the card, so nothing here needs to be able to.
    if let Err(why) = mount_boot_partition(k, false) {
        // Not a refusal: an emulator has no boot partition at all (`ADR-026`
        // §4), and three callers before this one have already said so.
        let outcome = Carried::Nothing;
        k.say(&format!("  no boot partition to read ({why})"));
        k.say(&format!("  {}", outcome.summary()));
        return outcome;
    }

    let outcome = carry_from_mounted(k);
    umount_boot_partition(k);
    k.say(&format!("  {}", outcome.summary()));
    outcome
}

/// The part that assumes the boot partition is mounted.
fn carry_from_mounted(k: &mut dyn Kernel) -> Carried {
    let entries = match k.read_dir(BOOT_PARTITION_MOUNT) {
        Ok(entries) => entries,
        Err(errno) => {
            return Carried::Refused(format!(
                "{BOOT_PARTITION_MOUNT} could not be listed: {errno}"
            ));
        }
    };

    // **Sorted, and filtered by the writer's own rule.** A boot partition holds
    // a kernel, a device tree, four generated text files and possibly two
    // secrets; `carried::product_of` answers `None` for every one of them, so
    // there is no list of exceptions here to fall out of step with that one.
    let mut products: Vec<String> = entries
        .iter()
        .filter_map(|name| carried::product_of(name))
        .collect();
    products.sort();
    products.dedup();
    if products.is_empty() {
        return Carried::Nothing;
    }

    if let Err(errno) = k.mkdir_private(CARRIED_DIR) {
        return Carried::Refused(format!("cannot create {CARRIED_DIR}: {errno}"));
    }

    let mut written = 0usize;
    let mut kept = 0usize;
    for product in products {
        let record = carried::record_dir(&product);
        // **The record directory is the whole of the idempotence.** It is
        // created here and removed by nobody, so a payload that has already
        // been carried — whether it is still waiting or has been installed and
        // removed — is skipped. Without this a card would reinstall its
        // applications on every boot for ever, which is
        // `cmd/rivet-sysd`'s `deploy_request` hazard with a `read_dir` in place
        // of a `stat`.
        if k.read_dir(&record).is_ok() {
            kept = kept.saturating_add(1);
            continue;
        }

        let on_card = format!("{BOOT_PARTITION_MOUNT}/{}", carried::card_file(&product));

        // `stat` before `read` — R-2. A cap checked only after the whole file
        // is already in memory is not a cap on the allocation, and PID 1 is
        // the process that must not be the reason a board does not boot
        // (`ADR-017` §3). A `stat` failure of any kind falls through to the
        // read below, which already reports every one of those cases.
        if let Ok(size) = k.file_size(&on_card)
            && size as usize > carried::MAX_PAYLOAD_BYTES
        {
            return Carried::Refused(format!(
                "{on_card} is {size} bytes and a carried payload may be at most {}. Nothing was \
                 written and nothing on the card was touched",
                carried::MAX_PAYLOAD_BYTES
            ));
        }

        let bytes = match k.read_bytes(&on_card) {
            Ok(bytes) => bytes,
            Err(errno) => {
                return Carried::Refused(format!(
                    "{on_card} could not be read: {errno}. Nothing has been removed from the \
                     card, so the next boot tries again"
                ));
            }
        };
        // **A cap, because this is PID 1 and it has just read into memory.**
        // The boot partition is 256 MiB, so this is not the only bound — but a
        // card whose payload is a quarter of the partition is a build that went
        // wrong, and PID 1 is the process that must not be the reason a board
        // does not boot (`ADR-017` §3). Checked again here as a backstop for
        // whatever `file_size` above could not answer.
        if bytes.len() > carried::MAX_PAYLOAD_BYTES {
            return Carried::Refused(format!(
                "{on_card} is {} bytes and a carried payload may be at most {}. Nothing was \
                 written and nothing on the card was touched",
                bytes.len(),
                carried::MAX_PAYLOAD_BYTES
            ));
        }

        if let Err(errno) = k.mkdir_private(&record) {
            return Carried::Refused(format!("cannot create {record}: {errno}"));
        }
        // `write_secret` and not `write_file`: a payload is not a secret, but it
        // is a program the machine is about to verify and run, and a file an
        // unprivileged process could replace between here and `cmd/rivet-deploy`
        // would be a signature check performed on bytes nobody chose.
        let at = carried::payload_path(&product);
        if let Err(errno) = k.write_secret(&at, &bytes) {
            return Carried::Refused(format!(
                "cannot write {at}: {errno}. The payload is still on the card and the next boot \
                 tries again"
            ));
        }
        written = written.saturating_add(1);
    }

    // Flushed before anything else in the boot happens, so that a power cut
    // between here and the supervisor's first convergence leaves a payload that
    // is either wholly on `/state` or not there at all. Nothing is destroyed
    // here, so this is not the safety ordering [`crate::staging`] has — it is
    // the ordinary one.
    k.sync();

    Carried::Landed { written, kept }
}

