//! **What pid 1 can answer, and the far larger list it refuses** — the
//! `Board` behind `ADR-035` §8A.1.2's break-glass console.
//!
//! # The refusals are the design, not the leftovers
//!
//! `rivet_core::operation::Board` is four ports and several dozen methods,
//! because a Rivet answered over the control plane is a whole machine:
//! `rivet-mcpd` reaches adapters for netlink, `/proc`, `/sys`, the trust store,
//! the package format and the supervisor's socket.
//!
//! **Pid 1 must not grow any of that.** §8A.1.2: *"pid 1's failure mode is the
//! machine, so what goes in it must be small"* — and a break-glass endpoint
//! that pulled in the network stack to answer `get network` would have made the
//! process that must never die depend on the subsystem most likely to be the
//! reason somebody reached for break-glass.
//!
//! So this implements every one of them and **answers six**. The rest return
//! [`refused`], which is the same sentence every time and names where the
//! answer does live. §8A.1.2 asked for exactly that: *"refusing anything it
//! cannot serve."*
//!
//! # What it answers, and why those
//!
//! The set is *the 2am repair*, and nothing wider:
//!
//! | Answered | Because |
//! | :-- | :-- |
//! | `read_desired_state` | you cannot fix a document you cannot read |
//! | `read_file`, `can_write_file`, `write_file` | `apply edit` is the repair itself |
//! | `running_slot` | which half of the card is running decides what to do next |
//! | `can_arm_trial_boot`, `arm_trial_boot` | boot the other slot once, without committing |
//! | `can_request_stop`, `request_stop` | restart or power off, which pid 1 already owns |
//! | `principal` | who did it, for the record |
//!
//! Every one of those is something pid 1 **already does** in the course of
//! being pid 1. Nothing here is a capability added for break-glass; it is the
//! capabilities pid 1 has, exposed over a cable.
//!
//! # What it deliberately does NOT answer
//!
//! `get status` is the sharpest of these, because it is the one an operator
//! will reach for first and its absence looks like an oversight. It is not:
//! **the supervisor answers it**, over a socket, and the whole premise of Tier
//! 1 is that the supervisor may be what failed. A `get status` from pid 1 would
//! either lie or hang, and §8A.1.2 chose pid 1 precisely to escape both.
//!
//! Installing is refused for a different reason: an install writes hundreds of
//! megabytes and `cmd/rivet-deploy` exists because *a session must not own that
//! write* (the maintainer's 2026-08-24 ruling). A break-glass console is the worst possible owner
//! of a long write — it is a cable somebody may unplug.

use rivet_core::deploy::Principal;
use rivet_core::operation::machine::{
    BootRead, CpuRead, Deployer, DeviceRead, Editor, FileRead, JournalRead, Machine, MachineError,
    MemoryRead, NetworkRead, Observed, OwnerIds, PathChange, PathStat, ProcessRead, Reboot,
    SlotRead, StorageRead, ThermalRead, TrustRead, Write, Wrote,
};
use rivet_core::state::Slot;
use rivet_core::value::Value;

use crate::kernel::{Kernel, RebootHow};

/// The one sentence every unserved method answers with.
///
/// **One sentence and not one per method**, because they all mean the same thing
/// and an operator reading the twelfth of them learns nothing from a twelfth
/// wording. It names the subject so the refusal is about the thing that was
/// asked, and it names where the answer lives.
fn refused(what: &str) -> MachineError {
    MachineError::Refused(format!(
        "BREAK-GLASS cannot answer `{what}`. This is pid 1 over the serial console (ADR-035 \
         §8A.1.2) and it serves only what it can do without the supervisor, the network or a \
         filesystem it has not already mounted: read and edit the desired-state document, say \
         which slot is running, arm a trial boot, restart, and power off. Everything else is \
         the CONTROL PLANE's — MCP over TLS on port 7777 — which is what to fix, and what this \
         console exists to help you fix."
    ))
}

/// The board pid 1 presents to the operation catalogue.
pub struct BreakGlassBoard<'k> {
    k: &'k mut dyn Kernel,
    /// The slot this boot is running, as pid 1 already determined it.
    ///
    /// **Passed in rather than read here.** Pid 1 works it out once during boot
    /// and a second reading could disagree with the first, which is exactly the
    /// `ADR-011` failure the whole project is built to avoid.
    slot: Slot,
    /// Set when an operation asked for the machine to stop or trial-boot.
    ///
    /// **Recorded rather than performed.** A `request_stop` that rebooted from
    /// inside the operation would never return an answer to the console, so the
    /// operator would watch the machine die without being told it agreed. Pid 1
    /// reads this after the reply is written.
    pub pending: Option<RebootHow>,
}

impl<'k> BreakGlassBoard<'k> {
    /// A board over pid 1's own kernel.
    pub fn new(k: &'k mut dyn Kernel, slot: Slot) -> Self {
        Self {
            k,
            slot,
            pending: None,
        }
    }
}

impl Machine for BreakGlassBoard<'_> {
    fn running_slot(&mut self) -> Result<Slot, MachineError> {
        Ok(self.slot)
    }

    fn can_arm_trial_boot(&mut self, _slot: Slot) -> Result<(), MachineError> {
        Ok(())
    }

    fn arm_trial_boot(&mut self, _slot: Slot) -> Result<Reboot, MachineError> {
        // `OneShot`, and that is the Raspberry Pi's answer rather than a
        // choice made here: `ADR-018` §5.1's one-shot lives in the reboot
        // request itself, so an ordinary restart would boot the default slot
        // and trial nothing.
        self.pending = Some(RebootHow::TrialRestart);
        Ok(Reboot::OneShot)
    }

    fn can_request_stop(
        &mut self,
        _stop: rivet_core::operation::machine::Stop,
    ) -> Result<(), MachineError> {
        Ok(())
    }

    fn request_stop(
        &mut self,
        stop: rivet_core::operation::machine::Stop,
    ) -> Result<(), MachineError> {
        self.pending = Some(match stop {
            rivet_core::operation::machine::Stop::PowerOff => RebootHow::PowerOff,
            rivet_core::operation::machine::Stop::Restart => RebootHow::Restart,
            rivet_core::operation::machine::Stop::TrialRestart => RebootHow::TrialRestart,
        });
        Ok(())
    }

    fn can_request_reload(&mut self) -> Result<(), MachineError> {
        Err(refused("apply reload"))
    }

    fn request_reload(&mut self) -> Result<(), MachineError> {
        Err(refused("apply reload"))
    }

    fn can_request_service_restart(
        &mut self,
        _service: &str,
    ) -> Result<rivet_core::operation::service_restart::Restarting, MachineError> {
        Err(refused("apply service-restart"))
    }

    fn request_service_restart(
        &mut self,
        _service: &str,
    ) -> Result<rivet_core::operation::service_restart::Restarting, MachineError> {
        Err(refused("apply service-restart"))
    }
}

impl Observed for BreakGlassBoard<'_> {
    fn read_desired_state(&mut self) -> Result<Option<String>, MachineError> {
        match self.k.read_file(rivet_core::state::MACHINE_DOCUMENT) {
            Ok(text) => Ok(Some(text)),
            // **Absent is an answer, not a fault.** A first boot has no
            // document, and reporting that as an error would make an ordinary
            // state look like the thing break-glass was called for.
            Err(crate::kernel::Errno::ENOENT) => Ok(None),
            Err(e) => Err(MachineError::Refused(format!(
                "{} could not be read: {e}",
                rivet_core::state::MACHINE_DOCUMENT
            ))),
        }
    }

    fn read_journal(&mut self) -> Result<JournalRead, MachineError> {
        Err(refused("get log"))
    }
    fn read_filesystems(&mut self) -> Result<StorageRead, MachineError> {
        Err(refused("get storage"))
    }
    fn read_memory(&mut self) -> Result<MemoryRead, MachineError> {
        Err(refused("get memory"))
    }
    fn read_processes(&mut self) -> Result<ProcessRead, MachineError> {
        Err(refused("get process"))
    }
    fn read_cpu(&mut self) -> Result<CpuRead, MachineError> {
        Err(refused("get cpu"))
    }
    fn read_trust(&mut self) -> Result<TrustRead, MachineError> {
        Err(refused("get trust"))
    }
    fn read_thermal(&mut self) -> Result<ThermalRead, MachineError> {
        Err(refused("get thermal"))
    }
    fn read_boot(&mut self) -> Result<BootRead, MachineError> {
        Err(refused("get boot"))
    }
    fn read_slots(&mut self) -> Result<SlotRead, MachineError> {
        // **`get slot` and `running_slot` are different questions.** This one
        // compares two authorities on the card and pid 1 holds neither.
        Err(refused("get slot"))
    }
    fn read_network(&mut self) -> Result<NetworkRead, MachineError> {
        Err(refused("get network"))
    }
    fn read_devices(&mut self) -> Result<DeviceRead, MachineError> {
        Err(refused("get device"))
    }
    fn configuration_schema(&mut self) -> Value {
        // Not fallible in the trait, so the refusal has to be the value.
        Value::str(
            "BREAK-GLASS does not publish the schema; the control plane does (ADR-035 §8A.1.2)",
        )
    }
    fn supervisor_status(&mut self) -> Result<rivet_core::operation::status::Status, MachineError> {
        // The sharp one. See the module header: the supervisor answers this,
        // and Tier 1 exists for the case where the supervisor is what failed.
        Err(refused("get status"))
    }
}

impl Deployer for BreakGlassBoard<'_> {
    fn can_request_install(
        &mut self,
        _digest: &rivet_core::artifact::Digest,
    ) -> Result<rivet_core::operation::Held, MachineError> {
        Err(refused("apply install"))
    }
    fn request_install(
        &mut self,
        _digest: &rivet_core::artifact::Digest,
    ) -> Result<rivet_core::operation::Held, MachineError> {
        Err(refused("apply install"))
    }
    fn can_request_uninstall(&mut self, _name: &str) -> Result<(), MachineError> {
        Err(refused("apply uninstall"))
    }
    fn request_uninstall(&mut self, _name: &str, _purge: bool) -> Result<(), MachineError> {
        Err(refused("apply uninstall"))
    }
    // Refused for `apply uninstall`'s reason: the removal is the supervisor's,
    // and the premise of this console is that the supervisor may be what
    // failed.
    fn can_request_library_uninstall(
        &mut self,
        _selector: &rivet_core::uninstall_library::Selector,
    ) -> Result<rivet_core::uninstall_library::Removal, MachineError> {
        Err(refused("apply uninstall-library"))
    }
    fn request_library_uninstall(
        &mut self,
        _selector: &rivet_core::uninstall_library::Selector,
    ) -> Result<rivet_core::uninstall_library::Removal, MachineError> {
        Err(refused("apply uninstall-library"))
    }
    fn can_request_repo_delete(&mut self, _name: &str) -> Result<(), MachineError> {
        Err(refused("apply repo-delete"))
    }
    fn request_repo_delete(&mut self, _name: &str, _id: &str) -> Result<(), MachineError> {
        Err(refused("apply repo-delete"))
    }
}

impl Editor for BreakGlassBoard<'_> {
    fn principal(&mut self) -> Principal {
        // **Physical presence is the credential** — §8A.1. There is no
        // identity to name because the cable is the authentication, and a
        // record that invented one would be worse than one that says so.
        Principal::new(rivet_core::deploy::Origin::Console)
    }

    fn read_file(&mut self, path: &str) -> Result<FileRead, MachineError> {
        // **`missing_parents` is always empty here**, and that is a refusal in
        // disguise rather than an oversight: pid 1 does not walk a path to find
        // which of its parents are absent, so a creation that would need one
        // fails at the write with the kernel's own errno instead of being
        // planned. Break-glass repairs a document that already exists.
        match self.k.read_file(path) {
            Ok(text) => Ok(FileRead {
                contents: Some(text.into_bytes()),
                missing_parents: Vec::new(),
            }),
            Err(crate::kernel::Errno::ENOENT) => Ok(FileRead {
                contents: None,
                missing_parents: Vec::new(),
            }),
            Err(e) => Err(MachineError::Refused(format!(
                "{path} could not be read: {e}"
            ))),
        }
    }

    fn can_write_file(&mut self, _path: &str, _contents: &str) -> Result<(), MachineError> {
        Ok(())
    }

    fn write_file(&mut self, write: &Write<'_>) -> Result<Wrote, MachineError> {
        self.k
            .overwrite_in_place(write.path, write.contents.as_bytes())
            .map_err(|e| {
                MachineError::Refused(format!("{} could not be written: {e}", write.path))
            })?;
        // Nothing unrecorded: pid 1 has no journal of its own to fail to
        // write to, and saying so is different from saying nothing.
        Ok(Wrote { unrecorded: None })
    }

    fn stat_path(&mut self, _path: &str) -> Result<PathStat, MachineError> {
        Err(refused("get file"))
    }
    fn resolve_owner(
        &mut self,
        _user: Option<&str>,
        _group: Option<&str>,
    ) -> Result<OwnerIds, MachineError> {
        Err(refused("apply set-owner"))
    }
    fn can_change_path(&mut self, _change: &PathChange<'_>) -> Result<(), MachineError> {
        Err(refused("the filesystem verbs"))
    }
    fn change_path(&mut self, _change: &PathChange<'_>) -> Result<Wrote, MachineError> {
        Err(refused("the filesystem verbs"))
    }
}
