//! **The first domain port in this repository**, and the reason it is the first.
//!
//! `AGENTS.md` §3.1 draws the distinction this module sits on, and it is worth
//! quoting rather than paraphrasing because every other port here is the *other*
//! kind:
//!
//! > **Every port trait in this repository today is the second kind**, and that
//! > is correct rather than a drift. `BootMedium` carries an `Errno`, a mount and
//! > a filesystem type; `SendRecv` carries a netlink socket. **`rivet-core`
//! > could not name any of them without breaking its own rule.**
//! >
//! > **`rivet-core` owns no port traits yet for a reason, and it is not
//! > neglect.** No operation in the core has needed the world so far: `explain`
//! > answers from a const table. The first operation that must *read* something
//! > … is the one that will need a domain port, and that port goes in
//! > `rivet-core`.
//!
//! `crate::operation::trial` is that operation. It cannot answer from a const
//! table: *which slot is this machine running* is the firmware's fact
//! (`ADR-011`), and *arm the next boot* changes a file on a card. So this trait
//! exists, and the test §3.1 gives is the one to apply to it:
//!
//! > ask whether the *domain* is the thing that cannot do it, or whether an
//! > *adapter* is hiding a platform from its own tests. If the trait's method
//! > signatures mention anything the core is forbidden to name, you have
//! > answered it.
//!
//! Nothing below mentions a mount, an `Errno`, a device node, a partition
//! number, `libc`, or `tryboot`. It mentions [`Slot`], which is
//! `rivet-core`'s own — `ADR-018` §4's A and B, which `crate::state` already
//! owned. **That is what makes it a domain port rather than a second
//! `BootMedium`**, and it is why a UEFI machine, behind the same boot-slot port,
//! implements the same three methods without a FAT filesystem anywhere in sight.
//!
//! # This crate ships no implementor, and that is `ADR-014` §3
//!
//! The same posture `rivet-adapter-image`'s `port` module records. An
//! implementor would need `mount(2)`, and `rivet-core`'s `[dependencies]` table
//! is empty *precisely so the build refuses* rather than a human noticing
//! (`AGENTS.md` §3, gate 3). `cmd/rivet` supplies the only implementor there
//! is, over `rivet-adapter-image`'s `BootSlots` — one composition root, two
//! seams, and the platform behind both of them.
//!
//! # Why the error is two variants and not a string
//!
//! `ADR-006` §4.1: *"An agent branches on the code; a human reads the string."*
//! The two things an operator does next are genuinely different — fix the
//! request or the card, versus look at the hardware — so the port carries the
//! distinction rather than making the operation guess it back out of prose.
//! `crate::operation::trial` maps the two onto `E_UNSAFE` and
//! `E_MACHINE_REFUSED`.

use crate::artifact::Digest;
use crate::state::Slot;
use crate::value::Value;

/// Which reboot carries an arming, in the domain's words.
///
/// **A third spelling of a two-member enum, and each of the three is owned by a
/// different layer on purpose.** `cmd/rivet-initd`'s `RebootHow` is a *syscall*
/// vocabulary that also carries `PowerOff`; `rivet_adapter_image::ArmedReboot`
/// is the *image port*'s, and its own documentation explains why it did not
/// simply reuse `RebootHow`. This one is the *domain*'s, and it exists for the
/// same reason with the crate boundary making it structural rather than stylistic
/// — `rivet-adapter-image` depends on `rivet-core` and `ADR-014` §3 makes
/// that direction permanent, so the core cannot name `ArmedReboot` even if it
/// wanted to. The conversion is two lines in `cmd/rivet`, and it is tested
/// there.
///
/// **Neither member says the machine has rebooted.** Nothing in Rivet's
/// administration surface reboots anything; see [`Machine::arm_trial_boot`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reboot {
    /// The arming is already recorded where the firmware will read it, so an
    /// ordinary restart picks it up. UEFI `BootNext`'s answer.
    Ordinary,
    /// The one-shot lives **in the reboot request itself**, so an ordinary
    /// restart would boot the default slot and trial nothing. The Raspberry Pi's
    /// answer — `ADR-018` §5.1's `reboot "0 tryboot"`.
    OneShot,
}

impl Reboot {
    /// The wire form, for a result an agent reads.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ordinary => "ordinary",
            Self::OneShot => "one-shot",
        }
    }

    /// What the operator has to arrange, in a sentence.
    ///
    /// The two must not read alike: an operator who performs the wrong one gets
    /// a board that comes back healthy having trialled nothing, which is the
    /// silent success this whole operation is built to avoid.
    pub const fn requirement(self) -> &'static str {
        match self {
            Self::Ordinary => "an ordinary restart is enough; the firmware already has the arming",
            Self::OneShot => {
                "the restart itself must carry the one-shot flag (ADR-018 §5.1's \
                 `reboot \"0 tryboot\"`); an ordinary restart would boot the default slot \
                 and trial nothing"
            }
        }
    }
}

impl core::fmt::Display for Reboot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// **How to stop this machine**, in the domain's words — `ADR-017` §3.1.10's
/// three-row table, said from the other end.
///
/// PID 1 owns stopping a Rivet machine (`ADR-017` §3.1.5 item 6) and the three
/// rows of §3.1.10's table are the whole of that contract. This enum is the
/// **request** half of it: what an authenticated principal (`ADR-028` §4) may ask
/// for through the administration surface (`ADR-006` §5).
///
/// **A fourth spelling of what is nearly the same enum, and the fourth is the
/// domain's.** `cmd/rivet-initd`'s `RebootHow` is a *syscall* vocabulary —
/// `RB_POWER_OFF`, `RB_AUTOBOOT`, `LINUX_REBOOT_CMD_RESTART2` — and its
/// `kernel::Sig` is the *signal* vocabulary that carries a request to it. Neither
/// is nameable here: `rivet-core`'s `[dependencies]` table is empty precisely so
/// the build refuses (`AGENTS.md` §3, gate 3), and a domain that could say
/// `SIGUSR1` would be a domain that had learned what a signal is.
///
/// **This is a request, not an act.** Nothing on [`Machine`] stops a machine; a
/// Rivet board is stopped by pid 1 and by nothing else. See
/// [`Machine::request_stop`] for what "delivered" is and is not worth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// Shut down and stay off.
    PowerOff,
    /// Shut down and come back, in whichever slot is the default.
    Restart,
    /// Shut down and come back **once** in the slot a trial boot armed, then
    /// return to the default on the next reset unless the boot is committed —
    /// `ADR-018` §5.1.
    TrialRestart,
}

impl Stop {
    /// Every way to stop a machine, in the order `ADR-017` §3.1.10 tabulates
    /// them. Written out rather than counted, because it is a contract.
    pub const ALL: &'static [Stop] = &[Stop::PowerOff, Stop::Restart, Stop::TrialRestart];

    /// The wire form, **and the subject an operator types**. One spelling, so a
    /// result can be fed back in as a request and so no adapter owns a second
    /// list of these words.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PowerOff => "power-off",
            Self::Restart => "restart",
            Self::TrialRestart => "trial-restart",
        }
    }

    /// What the machine will do, in a sentence an operator reads before
    /// confirming.
    ///
    /// The three must not read alike. An operator who asked for the wrong one
    /// gets a board that is off when they wanted it back, or — worse — one that
    /// came back on the default slot having trialled nothing, which looks
    /// exactly like success.
    pub const fn means(self) -> &'static str {
        match self {
            Self::PowerOff => {
                "the machine stops and stays off. It comes back only when power is removed and \
                 reapplied"
            }
            Self::Restart => {
                "the machine stops and comes back in the slot that is the default. A trial boot \
                 that was armed is NOT taken by this restart"
            }
            Self::TrialRestart => {
                "the machine stops and comes back ONCE in the slot a trial boot armed (ADR-018 \
                 §5.1). If that boot is not committed, the next reset returns to the default \
                 slot with no userspace involvement at all"
            }
        }
    }

    /// Whether this request carries the firmware's one-shot flag.
    ///
    /// [`Reboot::OneShot`] is what an arming *needs*; this is the request that
    /// supplies it. The two being separate types is `ADR-018` §5's separation of
    /// arming from restarting, and the pair is what an operator has to get right.
    pub const fn carries_the_one_shot(self) -> bool {
        matches!(self, Self::TrialRestart)
    }
}

impl core::fmt::Display for Stop {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why the machine did not do what was asked.
///
/// Closed, two variants, and the split is the one a caller acts on rather than
/// the one the implementation happens to have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MachineError {
    /// A safety interlock refused, or the machine is in a state this operation
    /// will not act on. **Nothing was changed**, and retrying will not help
    /// until the message's instruction is followed.
    Unsafe(String),
    /// The machine could not be read or changed. The message carries what the
    /// system said (`AGENTS.md` §5), not what it was expected to say.
    Refused(String),
}

impl MachineError {
    pub fn why(&self) -> &str {
        match self {
            Self::Unsafe(why) | Self::Refused(why) => why,
        }
    }
}

impl core::fmt::Display for MachineError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.why())
    }
}

impl std::error::Error for MachineError {}

/// The machine an operation administers, as the domain needs it.
///
/// Object-safe on purpose: every adapter passes `&mut dyn Machine` into
/// [`crate::operation::perform`], so the operations are compiled once and the
/// tests drive exactly the code a board runs.
///
/// **Deliberately not one trait per operation.** `ADR-006` §3.1's co-equality is
/// about adapters reaching operations, and a composition root that had to
/// assemble five ports to call `perform` would make a second adapter — the
/// network API of `ADR-006` §5.1 — expensive in exactly the way §3 forbids. It
/// is ~~three~~ **five** methods today because ~~one operation~~ **two
/// operations** need the world; the moment it is large enough to be
/// uncomfortable is the moment to split it, and that will be visible rather than
/// gradual.
///
/// **The count moved on 2026-08-23** and is struck rather than edited, because
/// `AGENTS.md` §7 is about exactly this sentence. The split it warned about has
/// not happened and the shape that would take it is visible: the first three
/// methods are `ADR-018` §4's slots and the last two are `ADR-017` §3.1.5 item
/// 6's shutdown, which is a different authority in a different ADR. A `Machine`
/// that grows a sixth method belonging to neither is the signal to cut it.
///
/// # The signal fired twice, and on 2026-08-24 it was acted on
///
/// `read_journal` was the sixth method and `read_filesystems` the
/// seventh, and neither belonged to the slots or to the shutdown. **Both now
/// live on [`Observed`]**, which is the trait this comment named before it
/// existed.
///
/// What is left is one authority: five methods, `ADR-018` §4's slots and
/// `ADR-017` §3.1.5 item 6's shutdown, every one of them changing the machine
/// and every one of them paired with a `can_` half because `ADR-006` §7's
/// plan-then-apply has to ask before it acts. **That pairing is now the
/// membership test.** A method with no `can_` half is an observation and
/// belongs on [`Observed`]; a method that needs one belongs here.
///
/// The objection recorded above — that assembling *five* ports to call
/// [`crate::operation::perform`] would make `ADR-006` §5.1's network adapter
/// expensive — stands, and is why the cut stopped at two. `perform` takes a
/// `&mut dyn Machine` and a `&mut dyn Observed`, and the next read-only fact
/// joins `Observed` instead of becoming a third parameter.
pub trait Machine {
    /// Which of `ADR-018` §4's two slots this machine is running.
    ///
    /// Read from the firmware at the point of use, never cached: `AGENTS.md` §5
    /// — *"the kernel is the authority for system state; never cache its
    /// verdicts"* — and here the authority is one layer below the kernel again.
    fn running_slot(&mut self) -> Result<Slot, MachineError>;

    /// Refuse now, **changing nothing**, if arming `slot` would be refused.
    ///
    /// `ADR-006` §6.2's dry run. ~~For the one operation that has one. §6.5's
    /// framework does not exist (`ADR-006` §8.1), so this is the property
    /// supplied by hand rather than by a type that cannot compile without it,
    /// and saying which of those it is matters (`AGENTS.md` §3).~~
    ///
    /// **Struck 2026-08-26**, and it is the sentence the whole of
    /// [`crate::operation::plan`] answers: the framework exists, seven
    /// operations have one, and it *is* now a type that cannot compile without
    /// it. This method is what `crate::operation::trial`'s
    /// [`crate::operation::plan::Change::plan`] calls, and calling it is no
    /// longer something a caller can forget.
    fn can_arm_trial_boot(&mut self, slot: Slot) -> Result<(), MachineError>;

    /// Make the **next** boot a one-shot trial of `slot`, and return which kind
    /// of restart then carries it.
    ///
    /// **This does not reboot, and no Rivet operation does.** `ADR-018` §5
    /// separates the persistent half from the restart because on a UEFI machine
    /// they genuinely are separate, and because arming and stopping a machine
    /// are two acts an operator gets to sequence. There is no reboot on this
    /// trait, so it cannot be done by accident either.
    fn arm_trial_boot(&mut self, slot: Slot) -> Result<Reboot, MachineError>;

    /// Refuse now, **changing nothing and stopping nothing**, if [`Self::
    /// request_stop`] would be refused.
    ///
    /// `ADR-006` §6.2's dry run for the three stop subjects, supplied by hand
    /// for the same reason [`Self::can_arm_trial_boot`] is: §6.5's framework does
    /// not exist (`ADR-006` §8.1).
    ///
    /// **This is where the interlock that matters lives, and it must run on the
    /// plan path too.** An adapter reaching a machine that is not a Rivet
    /// machine has to refuse *before* it has been confirmed, not after — an agent
    /// that saw a plan and then a refusal would reasonably conclude the
    /// confirmation was the problem and re-issue it. Which implementor holds the
    /// interlock, and what it reads, is the adapter's: the domain knows only that
    /// stopping a machine can be refused for a reason that is not the operator's
    /// fault to fix twice.
    fn can_request_stop(&mut self, stop: Stop) -> Result<(), MachineError>;

    /// **Ask** this machine to stop. `ADR-017` §3.1.5 item 6 and §3.1.10.
    ///
    /// # `Ok(())` means the request was delivered, and nothing more
    ///
    /// It does **not** mean the machine stopped, and no implementor may make it
    /// mean that. On a Rivet board the request reaches pid 1, which then stops
    /// the supervisor, waits for it, syncs the filesystems and asks the kernel —
    /// a sequence that outlives the process that asked for it and that reports
    /// on the console rather than to a caller. `AGENTS.md` §5 is the rule this
    /// obeys: *report what the system said, not what it was expected to say*, and
    /// what the system said here is only *"received"*.
    ///
    /// The consequence reaches the operator rather than being left in this
    /// comment: `crate::operation::stop`'s result says that the machine has not
    /// stopped yet and that the console is the authority for whether it does.
    ///
    /// # It is not the arming, and it never arms
    ///
    /// [`Stop::TrialRestart`] carries the firmware's one-shot flag; it does not
    /// choose which slot the flag names. That is [`Self::arm_trial_boot`], and
    /// `ADR-018` §5 keeps the two apart because on a UEFI machine they genuinely
    /// are separate acts and because an operator gets to sequence them.
    fn request_stop(&mut self, stop: Stop) -> Result<(), MachineError>;

    /// Refuse now, **asking nothing**, if [`Self::request_reload`] would be
    /// refused. `ADR-032` §3.2, added 2026-08-26.
    ///
    /// The same shape as [`Self::can_request_stop`] and for the same reason: an
    /// operator shown a plan and then a refusal would reasonably conclude the
    /// confirmation was the problem and re-issue it.
    ///
    /// **What it must check is that there is a supervisor to ask**, which on a
    /// board means a process that is pid 1's child and is the supervisor. On
    /// anything else there is not, and that is a refusal rather than a success
    /// with nothing behind it.
    fn can_request_reload(&mut self) -> Result<(), MachineError>;

    /// **Ask the supervisor to re-read the desired-state document.**
    /// `ADR-032` §3.2.
    ///
    /// # `Ok(())` means the request was delivered, and nothing more
    ///
    /// [`Self::request_stop`]'s sentence, unchanged, and here the limit is the
    /// ADR's own: **a signal cannot carry a reply.** The operator learns what a
    /// reload did from the journal, after the fact — which `ADR-032` §3.2
    /// accepts for a reload and *"would not for anything else"*.
    ///
    /// # Why this is an operation at all
    ///
    /// Because the CLI is not the only thing that will ever ask. `ADR-006` §3.1
    /// makes every operation reachable from every adapter, and §3.3's MCP row is
    /// a second caller with no code of its own — an adapter is a projection of
    /// the catalogue. A reload delivered by a bare word in `cmd/rivet` would be
    /// reachable from the login shell and from nowhere else, which §3.1 calls
    /// *"a defect in that interface, not a feature of it"*.
    fn request_reload(&mut self) -> Result<(), MachineError>;

    /// **Whether a button can be pressed on this machine** — `ADR-012` §9.
    ///
    /// The interlock is *is there a launcher listening*. `ADR-012` §9.3 rules
    /// that with no launcher there are no kiosk applications and therefore
    /// nothing an input could mean, so this refuses rather than queueing —
    /// and a queue would be the silent failure this project keeps naming.
    /// # Why this one is DEFAULTED and the rest are not
    ///
    /// Every other method here is required, so that a new adapter cannot
    /// silently fail to answer a question the catalogue asks. This one has a
    /// default because it arrived late — `ADR-012` §9, 2026-09-01 — into a
    /// trait with a dozen implementors, most of them test stand-ins that model
    /// no kiosk and never will.
    ///
    /// **The default refuses**, saying so in its own words. An adapter that
    /// forgot to implement it therefore behaves exactly as a machine with no
    /// launcher does, which is the honest answer rather than a silent one.
    fn can_send_input(&mut self) -> Result<(), MachineError> {
        Err(MachineError::Refused(
            "this build cannot deliver input to a kiosk on this machine. `apply input` needs a \
             launcher listening on the socket it provides (ADR-012 §9.4), and an adapter that \
             does not implement this port has none"
                .to_string(),
        ))
    }

    /// **Press one button.** `action` is a word from
    /// [`crate::input::Action`], already parsed and validated by the operation;
    /// this port takes the wire form because an adapter writes bytes.
    ///
    /// Defaulted for [`Self::can_send_input`]'s reason, and to the same refusal
    /// — a board that cannot say whether it could press cannot press.
    fn send_input(&mut self, action: &str) -> Result<(), MachineError> {
        let _ = action;
        self.can_send_input()
    }

    /// **What the supervisor WOULD do about one service**, changing nothing.
    /// `ADR-017` §6.3.2, added 2026-08-29.
    ///
    /// # It returns the preview rather than `Ok(())`, and that is the point
    ///
    /// Every other interlock on this port answers *may I* and nothing else,
    /// because the caller already knows what it is about to do. This one does
    /// not: what a restart takes depends on the state the supervisor is holding
    /// the service in, and **only the supervisor knows that**. A plan built from
    /// anything else would be the CLI's guess at a running machine, which is
    /// `ADR-011` §3 with the two answers a page apart.
    ///
    /// So the interlock *is* the plan: this sends `ADR-006` §6.4's `plan` verb
    /// across the supervisor's control surface and hands back the answer, and
    /// `crate::operation::service_restart::RestartService::plan` builds the
    /// operator's plan out of it. The refusals therefore happen here, before the
    /// confirmation is typed — §6.3.2's *"the refusals are also trivially
    /// writable"* reaching the one moment an operator can still choose
    /// differently.
    ///
    /// # Errors
    ///
    /// [`MachineError::Unsafe`] when the document disables the service or the
    /// machine is shutting down — neither changes on a retry.
    /// [`MachineError::Refused`] when there is no such service, or the
    /// supervisor could not be reached.
    fn can_request_service_restart(
        &mut self,
        service: &str,
    ) -> Result<crate::operation::service_restart::Restarting, MachineError>;

    /// **Ask the supervisor to discard one service's state and converge
    /// again.** `ADR-017` §6.3.2.
    ///
    /// The answer says what the supervisor found and what it is doing about it.
    /// It does **not** say the service is back up: convergence happens on the
    /// supervisor's next pass with `ADR-017` §3.5's stop deadline underneath it,
    /// and `get status` is the authority for the outcome.
    ///
    /// **The answer may not arrive at all** when the service being restarted is
    /// the one carrying the request — see
    /// [`crate::operation::service_restart::caveat`], which is what the plan
    /// says before this is confirmed.
    ///
    /// # Errors
    ///
    /// [`Self::can_request_service_restart`]'s, and the same mapping. It is
    /// asked again here rather than trusted from the plan, because the state can
    /// change in between and the supervisor is the authority for it.
    fn request_service_restart(
        &mut self,
        service: &str,
    ) -> Result<crate::operation::service_restart::Restarting, MachineError>;

    /// **What the supervisor WOULD do about stopping one service, or releasing
    /// that stop**, changing nothing. `ADR-017` §6.
    ///
    /// [`Self::can_request_service_restart`]'s shape and for its reason: what a
    /// stop takes depends on the state the supervisor holds the service in, so
    /// the interlock *is* the supervisor's own answer to `plan`.
    ///
    /// # Why this one is DEFAULTED
    ///
    /// [`Self::can_send_input`]'s reason: it arrived into a trait with a dozen
    /// implementors, most of them test stand-ins that model no supervisor. **The
    /// default refuses**, so an adapter that does not implement it behaves as a
    /// machine with no supervisor to ask, which is the honest answer.
    ///
    /// # Errors
    ///
    /// [`MachineError::Unsafe`] for every refusal that will not change on a
    /// retry; [`MachineError::Refused`] for an unknown name or a supervisor that
    /// could not be reached.
    fn can_request_service_hold(
        &mut self,
        kind: crate::operation::service_hold::Kind,
        service: &str,
    ) -> Result<crate::operation::service_hold::Hold, MachineError> {
        Err(MachineError::Refused(format!(
            "this build cannot ask a supervisor to {kind} `{service}`: `apply {kind}` crosses \
             the supervisor's own socket (ADR-017 §6), and an adapter that does not implement \
             this port has none to ask. NOTHING WAS SENT."
        )))
    }

    /// **Ask the supervisor to stop one service and hold it stopped, or to
    /// release that hold.** `ADR-017` §6.
    ///
    /// The answer says what the supervisor found and what it is doing. It does
    /// **not** say the process has exited or started: that happens on the
    /// supervisor's next pass, and `get status` is the authority for it.
    ///
    /// # Errors
    ///
    /// [`Self::can_request_service_hold`]'s, asked again rather than trusted
    /// from the plan, because the state can change in between.
    fn request_service_hold(
        &mut self,
        kind: crate::operation::service_hold::Kind,
        service: &str,
    ) -> Result<crate::operation::service_hold::Hold, MachineError> {
        self.can_request_service_hold(kind, service)
    }

    /// **Ask the supervisor what resetting the clock floor would do**,
    /// changing nothing — `crate::operation::clock_floor`, `ADR-021` §3.
    ///
    /// The default refuses: an adapter with no supervisor socket has nobody
    /// to ask, and a floor reported reset that was not would be a success with
    /// nothing behind it.
    ///
    /// # Errors
    ///
    /// [`MachineError::Refused`], carrying the supervisor's refusal or why it
    /// could not be asked.
    fn can_request_clock_floor_reset(
        &mut self,
    ) -> Result<crate::operation::clock_floor::Floor, MachineError> {
        Err(MachineError::Refused(format!(
            "this build cannot ask a supervisor to reset the clock floor: `apply {}` crosses the \
             supervisor's own socket (ADR-021 §3), and an adapter that does not implement this \
             port has none to ask. NOTHING WAS SENT.",
            crate::operation::clock_floor::SUBJECT
        )))
    }

    /// **Ask the supervisor to make the running clock the floor.** The answer
    /// is what it now holds.
    ///
    /// # Errors
    ///
    /// [`Self::can_request_clock_floor_reset`]'s.
    fn request_clock_floor_reset(
        &mut self,
    ) -> Result<crate::operation::clock_floor::Floor, MachineError> {
        self.can_request_clock_floor_reset()
    }
}

/// ~~Both ports~~ **All three ports** at once, for the one caller that needs
/// them.
///
/// [`crate::operation::perform`] dispatches every verb, so it reaches
/// [`Machine`], [`Observed`] and [`Deployer`] alike — and an adapter's board is
/// a single object that answers all of them. Passing it as three `&mut` is
/// impossible and passing three objects would make a composition root assemble
/// ports it does not have, which is the cost `Machine`'s own documentation
/// warned about.
///
/// So the entry point takes one `&mut dyn Board` and **the operations take the
/// narrow ports**, reached by upcasting at the dispatch. `ADR-006` §3.1's
/// single entry point keeps its shape, adapters keep their one object, and
/// `get log` still cannot reboot anything.
///
/// The blanket implementation is what keeps this free: anything that is all
/// three is a `Board` already, and no adapter writes an `impl` for it.
///
/// **It became three on 2026-08-24**, when `apply install` arrived — see
/// [`Deployer`] for why that was a cut rather than two more methods on
/// [`Machine`]. The count moved and the *shape* did not, which is the property
/// this trait exists to give: every adapter and every test that already had one
/// object still has one object.
pub trait Board: Machine + Observed + Deployer + Editor {}

impl<T: Machine + Observed + Deployer + Editor + ?Sized> Board for T {}

/// **What this machine can be asked to install.** `ADR-015` §7.1, `ADR-029`
/// §14.
///
/// # Why this is a third port and not two more methods on [`Machine`]
///
/// Because [`Machine`]'s own documentation said to, in the sentence that has
/// now fired three times:
///
/// > What is left is one authority: five methods, `ADR-018` §4's slots and
/// > `ADR-017` §3.1.5 item 6's shutdown … **A `Machine` that grows a sixth
/// > method belonging to neither is the signal to cut it.**
///
/// An install belongs to neither. It does not choose which slot boots — that is
/// `arm_trial_boot`, which the *installer* performs at the end of `ADR-029`
/// §9.2 and which this port never touches — and it stops nothing. It is a third
/// authority: `cmd/rivet-deploy`, the supervised binary the maintainer's
/// 2026-08-24 ruling created precisely so that **no operator's connection owns a
/// 640 MiB write**: a caller that hangs up mid-install must not be able to end
/// one.
///
/// The membership test [`Machine`] states is satisfied here too, and both halves
/// of it: these methods change the machine, and they come in a `can_`/do pair
/// because `ADR-006` §7's plan-then-apply has to ask before it acts. So this is
/// not [`Observed`] either. Three ports, three authorities, and the objection
/// [`Machine`] recorded — that a composition root assembling *five* ports would
/// make `ADR-006` §5.1's network adapter expensive — is answered the same way it
/// was at two: [`Board`] is what an adapter implements, and it is still one
/// object.
///
/// # Nothing here installs anything, and that is the whole point
///
/// [`Deployer::request_install`] delivers a request and returns. The maintainer's
/// 2026-08-24 install-request ruling:
///
/// > **`rivet apply install <path> confirm` hands over the path, answers, and
/// > returns — before a byte is written.** … An answer that waited would hand
/// > the hazard straight back.
///
/// So `Ok(())` means *delivered*, exactly as [`Machine::request_stop`]'s does,
/// and for a sharper version of the same reason: what happens next takes
/// minutes, writes 640 MiB, and reports on a journal this process is not
/// reading. `crate::operation::install` is what says so to the operator.
/// What this machine's artifact store holds under a digest.
///
/// **The answer to a resolution, and it is deliberately thin.** One field: the
/// `ADR-027` name the artifact arrived under. Not its size, not its product,
/// not its kind — the store could report all three and two of them would be
/// this crate's *reading* of a name rather than the machine's knowledge, which
/// is the distinction `crate::artifact::parse_name` is careful about. The name
/// is what the machine actually has, and it is what an operator needs in front
/// of them before they type `confirm`: a digest is sixty-four characters that
/// say nothing about what is in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    /// The file name inside `/state/artifacts/objects/<digest>/`.
    pub name: String,
}

pub trait Deployer {
    /// Refuse now, **delivering nothing**, if [`Self::request_install`] would be
    /// refused.
    ///
    /// `ADR-006` §6.2's dry run. ~~Supplied by hand for the reason
    /// [`Machine::can_arm_trial_boot`] is: §6.5's framework does not exist
    /// (`ADR-006` §8.1).~~ **Struck 2026-08-26 — the framework exists**
    /// ([`crate::operation::plan`]), and this port is what it calls while
    /// planning. Nothing about the method changed; what changed is that a
    /// caller can no longer forget to reach it.
    ///
    /// **It must run on the plan path**, which is [`Machine::can_request_stop`]'s
    /// argument and applies here unchanged: an agent shown a plan and then a
    /// refusal would reasonably conclude the confirmation was the problem and
    /// re-issue it.
    ///
    /// # What it does NOT check, and this is load-bearing
    ///
    /// **Whether the payload is any good.** Not the signature, not the digests,
    /// not the product, not the architecture — none of it, and not even that the
    /// file exists. `ADR-029` §11's argument is why:
    ///
    /// > three formats means three verifiers each with a third of the scrutiny
    ///
    /// A check written here would be a **second implementation** of reading the
    /// container, reaching a verdict the installer would then reach again by
    /// other code. Two verifiers that can disagree is the thing `ADR-029` §11
    /// rules out, and the weaker of the two is the one that would eventually be
    /// believed. So the payload is opaque on this side of the seam, and
    /// everything about it is refused *by the installer*, in the journal, under
    /// one correlation identifier.
    ///
    /// **The store's door does not change that, and the difference is worth
    /// stating** (`ADR-052` §7, 2026-09-12). The door verifies an artifact
    /// before it is stored by calling `crate::package::verify` — the installer's
    /// own implementation, not a second one. This port could call it too, and
    /// does not: the store has already asked, and the installer asks again
    /// against the trust store in force when it runs, which is the verdict that
    /// governs.
    ///
    /// What may be refused here is what is true of the **machine**: that this
    /// build cannot reach an installer at all, that the request cannot be
    /// delivered, or — since 2026-09-12 — that **this machine's artifact store
    /// does not hold the digest**.
    ///
    /// # The third refusal is not a fourth verifier
    ///
    /// Resolving a digest is a `readdir` on `/state/artifacts/objects/<digest>`
    /// and nothing else. It answers *does this machine have these bytes*, which
    /// is a question about the machine, and it opens nothing: the object's
    /// contents are as opaque here as a path's were. What it buys is the whole
    /// of `ADR-052`'s rule — an install can only name something that arrived
    /// through the transfer that hashed it.
    fn can_request_install(&mut self, digest: &Digest) -> Result<Held, MachineError>;

    /// **Ask** this machine to install `payload`. `ADR-015` §7.1, `ADR-029`
    /// §9.2.
    ///
    /// # `Ok(())` means the request was delivered, and nothing more
    ///
    /// It does **not** mean anything was installed, and no implementor may make
    /// it mean that — the same contract [`Machine::request_stop`] carries, and
    /// `ADR-006` §4.2 is what makes it enforceable rather than aspirational:
    /// *a request that was not delivered is a refusal, never a result with a
    /// `false` in it.*
    ///
    /// The reason is the ruling that created the installer. An update writes
    /// 640 MiB; the operator's session must not own that write, so this returns
    /// before the first byte and the account of what happened afterwards is the
    /// journal's. `crate::operation::install`'s `get` half is how it is read
    /// back.
    ///
    /// # `digest` is content, and it used to be a path
    ///
    /// **Changed 2026-09-12, `ADR-052` §7.** It was a path: *"a path says where
    /// to read the thing from rather than what it is"* — which was exactly the
    /// problem. A path could name anything on `/state`, so every artifact that
    /// ever reached a machine had a route onto it that the fleet repository
    /// never saw. A digest names the bytes, and the only bytes this machine has
    /// under a digest are the ones a transfer hashed on the way in.
    ///
    /// The **name** comes back rather than being passed in, for the same
    /// reason: the caller knows what it asked for and the machine knows what it
    /// holds, and an operator confirming an install should be reading the
    /// machine's answer.
    fn request_install(&mut self, digest: &Digest) -> Result<Held, MachineError>;

    /// Everything [`Self::request_uninstall`] checks, writing nothing.
    fn can_request_uninstall(&mut self, name: &str) -> Result<(), MachineError>;

    /// **Ask** this machine to remove an installed application.
    /// `crate::uninstall`, ruled 2026-08-25.
    ///
    /// `Ok(())` means *delivered*, exactly as [`Self::request_install`]'s does.
    /// What happens next is the **supervisor's**, and it is not instantaneous:
    /// a running application is stopped first, through the machinery
    /// `ADR-017` §3.5 already has, and only then is the tree removed. So this
    /// returns before anything is gone — `rivet get log` is where the outcome
    /// appears, on a record whose `svc` is `rivet-sysd`. (**Corrected
    /// 2026-08-30:** this said `rivet get log rivet-sysd`, which selects a BOOT
    /// of that name. `get log` has no service filter.)
    ///
    /// `purge` also removes `/state/app/<name>/data`. **It is `false` unless an
    /// operator typed the word**, ruled 2026-08-25: removing a program is not
    /// consent to discard what it wrote.
    fn request_uninstall(&mut self, name: &str, purge: bool) -> Result<(), MachineError>;

    /// Everything [`Self::request_library_uninstall`] checks, writing nothing —
    /// and **what it would remove**, because the plan names every build and
    /// every application that loses an addon.
    ///
    /// The decision is `crate::uninstall_library::survey`'s, which the
    /// supervisor makes again before it removes anything: this answer is the
    /// machine as it stood when it was asked, and the supervisor's is the one
    /// that governs.
    ///
    /// # Errors
    ///
    /// [`MachineError::Unsafe`] when an installed application declares a build
    /// this would remove; [`MachineError::Refused`] when nothing matches, when
    /// what is installed cannot be read, or when a request cannot be delivered.
    fn can_request_library_uninstall(
        &mut self,
        selector: &crate::uninstall_library::Selector,
    ) -> Result<crate::uninstall_library::Removal, MachineError>;

    /// **Ask** this machine to remove installed library builds —
    /// `crate::uninstall_library`.
    ///
    /// `Ok` means *delivered*, exactly as [`Self::request_uninstall`]'s does,
    /// and carries what the machine found when the request was written. The
    /// supervisor checks again and removes; `rivet get log` is where the
    /// outcome appears, on records whose `svc` is `rivet-sysd`.
    ///
    /// # Errors
    ///
    /// As [`Self::can_request_library_uninstall`], and a request that could not
    /// be written.
    fn request_library_uninstall(
        &mut self,
        selector: &crate::uninstall_library::Selector,
    ) -> Result<crate::uninstall_library::Removal, MachineError>;

    /// Whether this machine could be asked to **destroy a repository** —
    /// `crate::repo_delete`, ruled 2026-08-26.
    ///
    /// Answers whether the *request* can be delivered, and nothing more. The
    /// two conditions that decide whether it will be *honoured* — that the
    /// repository is no longer declared, and that the directory carries the
    /// identifier the operator named — are the supervisor's, because only the
    /// machine can see its own document and its own disk. A `can_` that
    /// pretended to check them would be answering a question it cannot see.
    ///
    /// # Errors
    ///
    /// [`MachineError`], as every other port here.
    fn can_request_repo_delete(&mut self, name: &str) -> Result<(), MachineError>;

    /// **Ask** this machine to destroy a repository.
    ///
    /// `Ok(())` means *delivered*, exactly as [`Self::request_uninstall`]'s
    /// does — and here the gap between delivered and done is wider than
    /// anywhere else in this port, because the supervisor may refuse outright.
    /// `rivet get log` is where the outcome appears, on a record whose `svc`
    /// is `rivet-sysd` — that operation has no service filter, so a service name
    /// as a word would select a boot (corrected 2026-08-30).
    ///
    /// `id` is the identifier the operator expects to destroy, carried so the
    /// machine can compare it to the one in the directory. A name is a label
    /// and labels get reused (`ADR-025` §4.1).
    ///
    /// # Errors
    ///
    /// [`MachineError`], as every other port here.
    fn request_repo_delete(&mut self, name: &str, id: &str) -> Result<(), MachineError>;
}

/// **One file on this machine, read and replaced whole.** `ADR-012` §2, and
/// `ADR-006` §8.9's ruling of 2026-08-24.
///
/// # Why this is a fourth port and not two more methods on [`Machine`]
///
/// [`Machine`]'s own documentation is the rule and it has now fired three
/// times:
///
/// > What is left is one authority: five methods, `ADR-018` §4's slots and
/// > `ADR-017` §3.1.5 item 6's shutdown … **A `Machine` that grows a sixth
/// > method belonging to neither is the signal to cut it.**
///
/// Editing a file belongs to neither. It does not choose which slot boots and
/// it stops nothing. It is not [`Observed`] either, by that trait's own
/// membership test: [`Self::write_file`] changes the machine and comes in a
/// `can_`/do pair, because `ADR-006` §7's plan-then-apply has to ask before it
/// acts.
///
/// # What is here and what is deliberately not
///
/// **Here:** the bytes, the parents that are missing, and the whole-file
/// replacement. Nothing more, and the shortness is the design — `ADR-006` §8.9
/// rules that this operation must grow *refusals* rather than expressiveness,
/// and a port with a `seek` or an `insert_at_line` would be the first step
/// towards being `sed`.
///
/// **Not here: the search, the counting, and every refusal that follows from
/// them.** Whether `--expect` occurs exactly once is the safety primitive of the
/// whole operation, and it is decided in `crate::operation::edit` — which is
/// what makes it testable with no filesystem at all, as `ADR-014` §3.1 requires
/// of this crate.
///
/// **Not here: bytes-to-text.** [`FileRead::contents`] is a `Vec<u8>` and the
/// domain decides what a file that is not UTF-8 means. An adapter that returned
/// a `String` would have had to guess — lossy, or a refusal in the wrong layer —
/// and `AGENTS.md` §5 forbids exactly that guess.
///
/// # It names nothing this crate is forbidden to name
///
/// `AGENTS.md` §3.1's test for a domain port. No `Errno`, no mode bits on the
/// interface, no `O_APPEND`, no mount. A path, some bytes, and a [`Principal`]
/// that `crate::deploy` already owned.
///
/// [`Principal`]: crate::deploy::Principal
pub trait Editor {
    /// **What this process can honestly say about who is asking.**
    /// `ADR-028` §4.5.
    ///
    /// The same [`crate::deploy::Principal`] `apply install` records, reached
    /// through the port rather than read here, because reading an environment is
    /// I/O and `rivet-core` does none. **A second notion of who is speaking
    /// would be `ADR-011`'s failure in the place it costs most** — an edit and an
    /// install attributed to the same session under two different names.
    ///
    /// It is on this port and not a parameter of [`Self::write_file`] because
    /// the **plan** must say who would be recorded, before anything is written.
    /// An operator who discovers their name in the journal afterwards was not
    /// told.
    fn principal(&mut self) -> crate::deploy::Principal;

    /// The bytes at `path`, and what is missing on the way to it.
    ///
    /// **A file that is not there is `Ok` with no contents, never an error.**
    /// `ADR-006` §8.9 rules that `apply edit` may create, so *"there is nothing
    /// here"* is an ordinary answer and one the plan reports. What must be an
    /// error is a path that exists and cannot be read — the distinction
    /// `crate::operation::log`'s reader draws, for the same reason: reading the
    /// second as the first would replace a file whose contents were never seen.
    fn read_file(&mut self, path: &str) -> Result<FileRead, MachineError>;

    /// Up to `len` bytes of `path`, starting at `at`, and the file's true
    /// size as this same read observed it — a POSITIONED read, never the
    /// whole file.
    ///
    /// **Added so `rivet get file` need not allocate a file it was only
    /// asked to look at part of.** `ADR-006` §8's cap bounds one ANSWER; before
    /// this existed, the only way to enforce that was [`Self::read_file`] —
    /// the whole file — followed by discarding what did not fit, which is
    /// exactly the allocation a caller windowing a large file is trying to
    /// avoid, on a board with no swap.
    ///
    /// `Ok(None)` when nothing is at `path`, on [`Self::read_file`]'s
    /// reasoning: *"there is nothing here"* is an ordinary answer and never
    /// an error. `total_bytes` is **not a stat taken earlier and carried
    /// along** — it is this read's own answer, fresh, because a caller
    /// computing `content_complete` against an older number would be
    /// trusting a verdict this call is in a position to give directly.
    ///
    /// An `at` at or past the end of the file is not this method's error to
    /// raise: the answer is an empty window and the true (smaller) size, and
    /// the caller is the one that knows what "past the end" means for it.
    ///
    /// # Errors
    ///
    /// [`MachineError`], as [`Self::read_file`].
    fn read_file_window(
        &mut self,
        path: &str,
        at: usize,
        len: usize,
    ) -> Result<Option<FileWindow>, MachineError> {
        // **The default is today's behaviour, unabridged: read the whole
        // file and slice it in memory.** Correct for any implementor that
        // does not override this — which is every implementor except
        // `cmd/rivet`'s real adapter and whatever forwards to it — and NOT
        // the fix: it is exactly the allocation this method exists to let
        // the real adapter stop making. A default exists here rather than a
        // required method on every implementor because most of them (every
        // test double, every refusing stub) do not touch a real filesystem
        // at all, and forcing each to restate this slicing would be seven
        // copies of one fact (`ADR-011`).
        let read = self.read_file(path)?;
        let Some(bytes) = read.contents else {
            return Ok(None);
        };
        let total_bytes = bytes.len();
        let start = at.min(total_bytes);
        let end = start.saturating_add(len).min(total_bytes);
        Ok(Some(FileWindow {
            bytes: bytes[start..end].to_vec(),
            total_bytes,
        }))
    }

    /// Refuse now, **writing nothing**, if [`Self::write_file`] would be
    /// refused.
    ///
    /// `ADR-006` §6.2's dry run. ~~Supplied by hand for the reason
    /// [`Machine::can_arm_trial_boot`] is: §6.5's framework does not exist
    /// (`ADR-006` §8.1).~~ **Struck 2026-08-26 — the framework exists**
    /// ([`crate::operation::plan`]), and this port is what it calls while
    /// planning. Nothing about the method changed; what changed is that a
    /// caller can no longer forget to reach it.
    ///
    /// **It must run on the plan path**, which is [`Machine::can_request_stop`]'s
    /// argument unchanged: an agent shown a plan and then a refusal would
    /// reasonably conclude the confirmation was the problem and re-issue it.
    ///
    /// **`contents` is what WOULD be written**, and it is here so a dry run can
    /// refuse everything the write would. `PLAN.md` §104.21: the guard that
    /// stops an unreadable configuration document reaching `/state` lives in an
    /// adapter — `rivet-core` cannot see `rivet-config`, the dependency runs the
    /// other way — so without the text this check could only ever ask *may I
    /// write here*, and `apply edit` would answer *yes* on the plan and refuse
    /// on the confirm. `ADR-006` §6.2 says a dry run refuses now what the act
    /// would refuse later, and a plan that is more optimistic than the write is
    /// the shape of surface this project spent 2026-08-31 being hurt by.
    fn can_write_file(&mut self, path: &str, contents: &str) -> Result<(), MachineError>;

    /// **Replace the file whole, atomically**, and record that it happened.
    ///
    /// # `Ok` means the bytes are on the card
    ///
    /// Unlike [`Machine::request_stop`] and [`Deployer::request_install`], this
    /// one really did the thing — there is no other process and no later
    /// verdict. An implementor that could not finish returns an error, and
    /// **never** an `Ok` describing a write that did not happen (`AGENTS.md`
    /// §3).
    ///
    /// # Atomicity is the contract, not an implementation note
    ///
    /// The file must never be observable half-written. A reader is not waiting
    /// for this process and may look at any moment — on a Rivet machine that
    /// reader is often the supervisor's next convergence pass. `cmd/rivet`'s
    /// implementor writes a temporary file, flushes it, renames it, and flushes
    /// the directory, which is the same sequence `crate::deploy`'s request uses
    /// and for the same reason.
    ///
    /// # The record is this port's, and a failure to write it is not a refusal
    ///
    /// [`Wrote::unrecorded`] carries what stopped the journal, and the write
    /// still succeeded. Refusing there would report *nothing was written* about
    /// a file that had been replaced, which is the worse of the two lies —
    /// `ADR-006` §4.2 points at the other direction and the same rule decides
    /// this one.
    fn write_file(&mut self, write: &Write<'_>) -> Result<Wrote, MachineError>;

    /// **Everything the machine knows about one path**, without reading it.
    ///
    /// `ADR-006` §8.10, ruled 2026-08-29. It is a *stat*, not a second reader:
    /// [`Self::read_file`] stays the one thing that produces bytes, because two
    /// readers of one file are two answers about whether it could be read
    /// (`ADR-011`).
    ///
    /// **A path with nothing at it is `Ok` with [`PathStat::kind`] `None`**, on
    /// [`Self::read_file`]'s reasoning: *"there is nothing here"* is an ordinary
    /// answer that `rivet get file` reports as `present: false`, and turning it
    /// into an error would make the absence indistinguishable from a failure to
    /// look.
    ///
    /// **It never follows a symbolic link.** A link is reported *as a link*,
    /// with its target, and the meta-info is the link's own. Following one would
    /// answer about a different path than the one that was asked about — and for
    /// the write subjects it would be a way out of `ADR-006` §8.10.4's permitted
    /// root, which is why [`PathStat::symlinked_ancestor`] exists.
    ///
    /// # Errors
    ///
    /// [`MachineError`], as every other port here — for a path that is there and
    /// cannot be examined, which is not the same as a path that is not there.
    fn stat_path(&mut self, path: &str) -> Result<PathStat, MachineError>;

    /// **A user and a group, by name or by number, resolved to both.**
    ///
    /// Asked through the port because resolving a name is I/O over `ADR-007`'s
    /// projected `/etc/passwd` and `/etc/group`, and a second copy of the
    /// identity table in `rivet-core` would be `ADR-011`'s failure in a place
    /// where the two answers are a uid.
    ///
    /// **It runs while planning**, so `apply set-owner`'s plan can say
    /// *"0 (root) becomes 1000 (app)"* — an operator must never confirm an
    /// identifier they have not seen resolved.
    ///
    /// A `None` argument is *not asked about* and comes back `None`; an unknown
    /// name is an error naming what is known, never a guess and never a silent
    /// numeric fallback.
    ///
    /// # Errors
    ///
    /// [`MachineError`] when a name given is not one this machine knows.
    fn resolve_owner(
        &mut self,
        user: Option<&str>,
        group: Option<&str>,
    ) -> Result<OwnerIds, MachineError>;

    /// Refuse now, **changing nothing**, if [`Self::change_path`] would be
    /// refused.
    ///
    /// `ADR-006` §6.2's dry run, reached by [`crate::operation::plan`] while
    /// planning — [`Self::can_write_file`]'s contract in full, and its argument
    /// unchanged: an agent shown a plan and then a refusal would reasonably
    /// conclude the confirmation was the problem and re-issue it.
    ///
    /// **This is where the checks only a machine can make live**, and `ADR-006`
    /// §8.10.4.3 names the one that matters: a component of the path that is a
    /// symbolic link is a way out of the permitted root that no test on a
    /// string can see.
    ///
    /// # Errors
    ///
    /// [`MachineError`], as every other port here.
    fn can_change_path(&mut self, change: &PathChange<'_>) -> Result<(), MachineError>;

    /// **Do one of `ADR-006` §8.10's acts**, and record that it happened.
    ///
    /// # Why one method over a closed enumeration, and why that is not the
    /// generic file operation §8.10.2 refuses
    ///
    /// The thing §8.9 forbids is a *surface* — one `apply file-op --op delete`
    /// whose single plan describes seven acts and whose single refusal list
    /// guards none of them properly. That surface does not exist: each act is
    /// its own subject, computes its own plan, and carries its own refusals, in
    /// `rivet-core`, which is exactly where §8.9 put them.
    ///
    /// **This is the seam where a platform does the thing**, and at that seam
    /// the seven acts really are one shape: a path, a verb, and whether it
    /// worked. Seven `can_`/do pairs here would be fourteen methods every fake
    /// in this workspace has to refuse, and the enumeration is *closed* — a
    /// caller cannot ask for an act this port does not name, which a pair of
    /// stringly-typed methods could not promise.
    ///
    /// # `Ok` means it happened
    ///
    /// Unlike [`Machine::request_stop`] and [`Deployer::request_install`], there
    /// is no other process and no later verdict. An implementor that could not
    /// finish returns an error and **never** an `Ok` describing an act that did
    /// not happen (`AGENTS.md` §3).
    ///
    /// # The record is this port's, and a failure to write it is not a refusal
    ///
    /// [`Self::write_file`]'s rule, and it bites harder here: a
    /// [`PathChange::RemoveFile`] that succeeded and could not be journalled has
    /// destroyed something, and reporting *nothing happened* about it would be
    /// the worst lie this port can tell.
    ///
    /// # Errors
    ///
    /// [`MachineError`], as every other port here.
    fn change_path(&mut self, change: &PathChange<'_>) -> Result<Wrote, MachineError>;

    /// **`len` bytes from the machine's own random source** — `getrandom(2)`
    /// on a board, which blocks until the kernel's pool is ready rather than
    /// returning weak bytes.
    ///
    /// A port rather than a parameter because `rivet-core` does no I/O, and
    /// minting a secret on the machine — `apply passkey-code` — needs one.
    ///
    /// **The default refuses**, so every test double and refusing stub in the
    /// tree keeps compiling and none of them can hand an operation bytes it
    /// would mistake for random ones (`AGENTS.md` §3). `cmd/rivet`'s real
    /// adapter overrides it.
    ///
    /// # Errors
    ///
    /// [`MachineError`] when there is no random source, or it failed.
    fn random_bytes(&mut self, len: usize) -> Result<Vec<u8>, MachineError> {
        Err(MachineError::Refused(format!(
            "this machine offers operations no random source, so {len} random bytes could not be \
             drawn and NOTHING WAS DONE"
        )))
    }

    /// **SHA-256 of `bytes`** — the fingerprint `rivet get fleet-root`
    /// answers beside a root's certificate (`ADR-063` §4).
    ///
    /// A port rather than code here because `rivet-core` takes no
    /// dependencies, and a hand-written digest in the crate every adapter
    /// trusts is the wrong place for a second implementation of one.
    ///
    /// **The default refuses**, so a test double cannot answer a fingerprint
    /// it did not compute. `cmd/rivet`'s real adapter overrides it.
    ///
    /// # Errors
    ///
    /// [`MachineError`] when this build has no digest to offer.
    fn sha256(&mut self, bytes: &[u8]) -> Result<[u8; 32], MachineError> {
        Err(MachineError::Refused(format!(
            "this machine offers operations no SHA-256, so {} bytes could not be fingerprinted",
            bytes.len()
        )))
    }
}

/// **What is at a path.** Four, closed — `ADR-006` §8.10.5.
///
/// `ADR-006` §6.4's closed-vocabulary rule applied to an answer rather than to a
/// command: an agent branches on these words, so they are part of the interface
/// contract and are never renamed. [`Self::Other`] is deliberately one word for
/// everything that is not the three an operator manages — a device node, a
/// socket, a fifo — because the useful thing to say about all of them is the
/// same: *this is not a file you edit, and no subject here will touch it*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    /// A regular file. The only kind `apply remove-file` will remove.
    File,
    Directory,
    /// A symbolic link. **Never followed** — [`PathStat::link_target`] says what
    /// it names, and `ADR-006` §8.10.4.3 says why the write subjects refuse one.
    Symlink,
    /// A device node, a socket, a fifo — anything else the kernel has.
    Other,
}

impl PathKind {
    /// The wire form, and the word a plan and a refusal carry. Part of the
    /// interface contract; never renamed.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
            Self::Symlink => "symlink",
            Self::Other => "other",
        }
    }

    /// Every kind, so a catalogue and a test can walk them.
    pub const ALL: &'static [Self] = &[Self::File, Self::Directory, Self::Symlink, Self::Other];
}

impl core::fmt::Display for PathKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One entry of a directory, as [`PathStat::entries`] carries it.
///
/// The **name** only, never a path: joining it to the directory is the caller's
/// and doing it here would produce two spellings of one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub name: String,
    pub kind: PathKind,
    /// Bytes for a file; for a directory it is whatever the filesystem says the
    /// directory itself weighs, which is **not** the size of what is in it.
    pub size_bytes: u64,
}

/// What an implementor of [`Editor::stat_path`] found.
///
/// **Every field is optional or defaulted, and that is not laziness.** A
/// development host has no owner names, a filesystem may have no modes, and a
/// path with nothing at it has none of them — a struct that made them mandatory
/// would force an implementor to invent a value, which is the guess `AGENTS.md`
/// §5 forbids.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PathStat {
    /// What is there, or `None` when nothing is. **`None` is not an error** —
    /// see [`Editor::stat_path`].
    pub kind: Option<PathKind>,
    /// Bytes, for a [`PathKind::File`]. Zero for everything else, which is what
    /// the filesystem says rather than a claim about contents.
    pub size_bytes: u64,
    /// The permission bits, masked to `0o7777`. `None` where the platform has
    /// no modes.
    pub mode: Option<u32>,
    pub uid: Option<u32>,
    /// The name for [`Self::uid`], where this machine knows one. `None` is *"no
    /// name for this number"* and never a substitute for the number.
    pub user: Option<String>,
    pub gid: Option<u32>,
    /// The name for [`Self::gid`]. See [`Self::user`].
    pub group: Option<String>,
    /// Milliseconds since the Unix epoch, or `None`. **A Pi has no RTC**
    /// (`ADR-021`), so a modification time on a machine whose clock has not been
    /// corrected is a real number about a wrong epoch — the answer says so
    /// rather than hiding it.
    pub modified_ms: Option<u64>,
    /// What a [`PathKind::Symlink`] names, verbatim and unresolved.
    pub link_target: Option<String>,
    /// The entries of a [`PathKind::Directory`], sorted by name, **capped** at
    /// [`crate::operation::filesystem::DIRECTORY_MAX_ENTRIES`].
    pub entries: Vec<DirectoryEntry>,
    /// How many entries there really are. Equal to `entries.len()` unless
    /// [`Self::entries_truncated`].
    pub entry_count: usize,
    /// Whether [`Self::entries`] is shorter than [`Self::entry_count`]. **A
    /// truncated list that did not say so is the same lie as a wrong one.**
    pub entries_truncated: bool,
    /// The first component on the way to this path that is itself a symbolic
    /// link, if any — `ADR-006` §8.10.4.3.
    ///
    /// It is here rather than only in [`Editor::can_change_path`] so that
    /// `rivet get file` can *report* it: a caller diagnosing why a write was
    /// refused should be able to see the reason without attempting the write.
    pub symlinked_ancestor: Option<String>,
    /// Whether something is mounted at this path. `false` where the platform
    /// cannot tell, which is why the destructive subjects treat it as one signal
    /// among several rather than as the guard.
    pub mount_point: bool,
}

impl PathStat {
    /// Whether anything at all is at the path.
    pub const fn present(&self) -> bool {
        self.kind.is_some()
    }

    /// Whether this is a regular file — the one kind `apply remove-file`
    /// removes and the one kind `rivet get file` will read.
    pub const fn is_file(&self) -> bool {
        matches!(self.kind, Some(PathKind::File))
    }

    /// Whether this is a directory.
    pub const fn is_directory(&self) -> bool {
        matches!(self.kind, Some(PathKind::Directory))
    }
}

/// A user and a group, resolved both ways — [`Editor::resolve_owner`].
///
/// **Both halves of each**, always, so that a plan can print `1000 (app)` and a
/// refusal can name whichever the operator did not give.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OwnerIds {
    pub uid: Option<u32>,
    pub user: Option<String>,
    pub gid: Option<u32>,
    pub group: Option<String>,
}

/// **One of `ADR-006` §8.10's acts, as [`Editor::change_path`] is asked for
/// it.**
///
/// Closed, and each variant carries exactly what *that* act needs and nothing
/// else — a `SetMode` has no `from` path and a `Rename` has no mode. The
/// `was_*` fields are for the **record**, never for the act: the machine does
/// not compare them, the domain already did, and a journal line that could not
/// say what a mode used to be would describe half an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathChange<'a> {
    /// Make a directory, and the parents named in the plan first — **outermost
    /// first**, exactly as [`FileRead::missing_parents`] orders them.
    CreateDirectory {
        path: &'a str,
        parents: &'a [String],
        mode: u32,
    },
    /// `chmod(2)`. `was` is for the record.
    SetMode {
        path: &'a str,
        mode: u32,
        was: u32,
    },
    /// `chown(2)`, **without following a symbolic link and without recursing**.
    /// A `None` half is left alone.
    SetOwner {
        path: &'a str,
        uid: Option<u32>,
        gid: Option<u32>,
        was_uid: Option<u32>,
        was_gid: Option<u32>,
    },
    /// `rename(2)`. The domain has already refused an existing `to`; an
    /// implementor must not rely on that alone, because `rename(2)` replaces
    /// silently and the window is real.
    Rename { from: &'a str, to: &'a str },
    /// `unlink(2)` **a regular file**. `was_bytes` is for the record — the size
    /// that is now gone is the one fact nobody can recover afterwards.
    RemoveFile { path: &'a str, was_bytes: u64 },
    /// `rmdir(2)` **an empty directory**. It is `rmdir` and not a walk: the
    /// kernel refuses a directory that is not empty, which is a second guard
    /// under the domain's own (`ADR-006` §8.10.12).
    RemoveDirectory { path: &'a str },
    /// **Put these exact bytes at `path`, owned by `uid`:`gid`, mode `mode`, in
    /// one act.** `crate::operation::install_secret`, 2026-09-01.
    ///
    /// # Why this is a `PathChange` and not a fourth [`Editor`] method
    ///
    /// Because it is a change to one path, which is what this enum is, and
    /// because [`Editor::can_change_path`] already carries the one check only a
    /// filesystem can make — a symbolic link on the way to the path. A new port
    /// method would have needed its own copy of that gate, and every
    /// implementor in the tree would have had to grow one. A variant needs an
    /// arm.
    ///
    /// # Why it is not [`Editor::write_file`] plus a [`SetOwner`](Self::SetOwner)
    ///
    /// `write_file` takes `&str` and decides the mode itself, so it can carry
    /// neither binary content nor an owner. The three-act sequence it forces —
    /// write, chown, chmod — is not equivalent, and the difference is the
    /// defect this variant exists to close:
    ///
    /// 1. **It cannot carry bytes.** An Ed25519 seed is not text and loses its
    ///    null bytes on the way through a `&str`.
    /// 2. **The file exists at its final name, readable by nobody but root,
    ///    between the acts.** An application started in that window reads
    ///    `EACCES` from its own certificate and reports it from inside itself,
    ///    at a moment nobody is watching.
    /// 3. **Each act is separately confirmable and separately failable.** Two
    ///    of three leaves a credential nothing can read and no record saying so.
    ///
    /// An implementor must therefore **set the ownership before the file
    /// reaches `path`**, which for a filesystem means writing a temporary,
    /// chowning that, and renaming it into place.
    ///
    /// # It replaces, deliberately
    ///
    /// Unlike `create-file`, which refuses an occupied path because creating is
    /// not clobbering, this one **replaces**: rotating a credential is the
    /// second thing anybody does with it, and a subject that could only ever
    /// install the first one would send an operator to `remove-file` with a
    /// live secret in hand. `was_bytes` is what the plan states and what the
    /// journal records, and it is the reason [`destroys`](Self::destroys) is
    /// true for a replacement.
    InstallSecret {
        path: &'a str,
        /// **Bytes, not text.** Never rendered into a diagnostic, a plan or a
        /// journal record — only its length is (`ADR-020` §4).
        contents: &'a [u8],
        mode: u32,
        uid: u32,
        gid: u32,
        /// What was there, in bytes. **Zero means nothing was there**, which is
        /// the first install; anything else is a replacement.
        was_bytes: u64,
    },
}

impl PathChange<'_> {
    /// The act's own word — the subject that asked for it. Part of the
    /// interface contract, and what a journal record carries.
    pub const fn act(&self) -> &'static str {
        match self {
            Self::CreateDirectory { .. } => "create-directory",
            Self::SetMode { .. } => "set-mode",
            Self::SetOwner { .. } => "set-owner",
            Self::Rename { .. } => "rename",
            Self::RemoveFile { .. } => "remove-file",
            Self::RemoveDirectory { .. } => "remove-directory",
            Self::InstallSecret { .. } => "install-secret",
        }
    }

    /// The path this act is *about* — the `from` of a rename, since that is the
    /// one that must already exist and the one every refusal names first.
    pub const fn path(&self) -> &str {
        match self {
            Self::CreateDirectory { path, .. }
            | Self::SetMode { path, .. }
            | Self::SetOwner { path, .. }
            | Self::RemoveFile { path, .. }
            | Self::RemoveDirectory { path }
            | Self::InstallSecret { path, .. } => path,
            Self::Rename { from, .. } => from,
        }
    }

    /// Whether this act **destroys** something. The one property a machine may
    /// legitimately branch on, and the reason it is a method rather than a
    /// `match` at each site.
    pub const fn destroys(&self) -> bool {
        match self {
            Self::RemoveFile { .. } | Self::RemoveDirectory { .. } => true,
            // **True only when it replaces something.** Installing a credential
            // where there was none destroys nothing; installing one over an
            // existing credential destroys a secret nothing can recover, and
            // that is exactly the act a reader scanning the journal for
            // severity wants to find at a glance. The first install and the
            // rotation are different events and this says which one happened.
            Self::InstallSecret { was_bytes, .. } => *was_bytes > 0,
            _ => false,
        }
    }
}

/// **An [`Editor`] that refuses everything**, for a test fake in another module
/// of this crate that is a [`Board`] and has no business editing files.
///
/// `crate::operation::install`'s and `crate::operation::trial`'s fakes reach
/// [`crate::operation::apply::perform`], which takes the whole [`Board`], so
/// each of them has to answer on this port to compile. **They refuse rather than
/// pretending** (`AGENTS.md` §3): a test in one of those modules that started
/// depending on an edit fails loudly instead of passing on a fiction.
///
/// A macro rather than a blanket implementation, because a blanket one would
/// silently give *every* future implementor a working-looking no-op — which is
/// the failure this whole file's "stubs refuse" rule is about.
#[cfg(test)]
macro_rules! refuses_to_edit {
    ($fake:ty) => {
        impl $crate::operation::machine::Editor for $fake {
            fn principal(&mut self) -> $crate::deploy::Principal {
                $crate::deploy::Principal::no_session()
            }

            fn read_file(
                &mut self,
                _path: &str,
            ) -> Result<
                $crate::operation::machine::FileRead,
                $crate::operation::machine::MachineError,
            > {
                Err($crate::operation::machine::MachineError::Refused(
                    concat!(
                        stringify!($fake),
                        " is a test stand-in that edits no files; an operation that reached \
                         this port has changed"
                    )
                    .to_string(),
                ))
            }

            fn can_write_file(
                &mut self,
                path: &str,
                contents: &str,
            ) -> Result<(), $crate::operation::machine::MachineError> {
                let _ = contents;
                self.read_file(path).map(|_| ())
            }

            fn write_file(
                &mut self,
                write: &$crate::operation::machine::Write<'_>,
            ) -> Result<
                $crate::operation::machine::Wrote,
                $crate::operation::machine::MachineError,
            > {
                self.read_file(write.path).map(|_| unreachable!())
            }

            // `ADR-006` §8.10's four, added 2026-08-29. Each refuses through
            // `read_file`, so the sentence an unexpected caller reads is the
            // same one and there is only ever one of it to keep true.
            fn stat_path(
                &mut self,
                path: &str,
            ) -> Result<
                $crate::operation::machine::PathStat,
                $crate::operation::machine::MachineError,
            > {
                self.read_file(path).map(|_| unreachable!())
            }

            fn resolve_owner(
                &mut self,
                _user: Option<&str>,
                _group: Option<&str>,
            ) -> Result<
                $crate::operation::machine::OwnerIds,
                $crate::operation::machine::MachineError,
            > {
                self.read_file("").map(|_| unreachable!())
            }

            fn can_change_path(
                &mut self,
                change: &$crate::operation::machine::PathChange<'_>,
            ) -> Result<(), $crate::operation::machine::MachineError> {
                self.read_file(change.path()).map(|_| ())
            }

            fn change_path(
                &mut self,
                change: &$crate::operation::machine::PathChange<'_>,
            ) -> Result<
                $crate::operation::machine::Wrote,
                $crate::operation::machine::MachineError,
            > {
                self.read_file(change.path()).map(|_| unreachable!())
            }
        }
    };
}

#[cfg(test)]
pub(crate) use refuses_to_edit;

/// **A [`Deployer`] that refuses everything**, for the same reason
/// [`refuses_to_edit`] exists and with the same posture.
///
/// Added 2026-08-26: `ADR-006` §6.5's framework has every mutating operation
/// take the whole [`Board`], so `crate::operation::stop`'s fake — which until
/// then only ever saw a [`Machine`] — has to answer on all four ports. Refusing
/// is the honest stand-in (`AGENTS.md` §3): a stop test that started installing
/// something fails loudly rather than passing on a fiction.
#[cfg(test)]
macro_rules! refuses_to_deploy {
    ($fake:ty) => {
        impl $fake {
            /// The one refusal this stand-in makes, written once.
            ///
            /// **An inherent method rather than five copies of a `concat!`.**
            /// Since the install port started answering with
            /// [`$crate::operation::machine::Held`] the five methods no longer
            /// share a return type, so the old shape — every method calling
            /// `can_request_install` — stopped type-checking. What they share
            /// is the *sentence*, and this is it.
            fn installs_nothing() -> $crate::operation::machine::MachineError {
                $crate::operation::machine::MachineError::Refused(
                    concat!(
                        stringify!($fake),
                        " is a test stand-in that installs nothing; an operation that reached \
                         this port has changed"
                    )
                    .to_string(),
                )
            }
        }

        impl $crate::operation::machine::Deployer for $fake {
            fn can_request_install(
                &mut self,
                _digest: &$crate::artifact::Digest,
            ) -> Result<
                $crate::operation::machine::Held,
                $crate::operation::machine::MachineError,
            > {
                Err(Self::installs_nothing())
            }

            fn request_install(
                &mut self,
                digest: &$crate::artifact::Digest,
            ) -> Result<
                $crate::operation::machine::Held,
                $crate::operation::machine::MachineError,
            > {
                self.can_request_install(digest)
            }

            fn can_request_uninstall(
                &mut self,
                _name: &str,
            ) -> Result<(), $crate::operation::machine::MachineError> {
                Err(Self::installs_nothing())
            }
            fn can_request_repo_delete(
                &mut self,
                _name: &str,
            ) -> Result<(), $crate::operation::machine::MachineError> {
                Err(Self::installs_nothing())
            }
            fn request_repo_delete(
                &mut self,
                _name: &str,
                _id: &str,
            ) -> Result<(), $crate::operation::machine::MachineError> {
                Err(Self::installs_nothing())
            }

            fn request_uninstall(
                &mut self,
                name: &str,
                _purge: bool,
            ) -> Result<(), $crate::operation::machine::MachineError> {
                self.can_request_uninstall(name)
            }

            fn can_request_library_uninstall(
                &mut self,
                _selector: &$crate::uninstall_library::Selector,
            ) -> Result<
                $crate::uninstall_library::Removal,
                $crate::operation::machine::MachineError,
            > {
                Err(Self::installs_nothing())
            }

            fn request_library_uninstall(
                &mut self,
                selector: &$crate::uninstall_library::Selector,
            ) -> Result<
                $crate::uninstall_library::Removal,
                $crate::operation::machine::MachineError,
            > {
                self.can_request_library_uninstall(selector)
            }
        }
    };
}

#[cfg(test)]
pub(crate) use refuses_to_deploy;

/// A [`Held`] for a test that needs one, named so a reader knows it is a
/// fixture.
#[cfg(test)]
impl Held {
    pub(crate) fn named(name: &str) -> Self {
        Self {
            name: name.to_string(),
        }
    }
}

/// **An [`Observed`] that refuses everything it can**, for [`refuses_to_edit`]'s
/// reason. Added 2026-08-26 with [`refuses_to_deploy`].
///
/// `configuration_schema` cannot refuse — it returns a value the build computes
/// rather than one a machine could decline — so it answers with a string that is
/// visibly not a schema. Loud rather than empty, so an assertion against real
/// schema content fails instead of passing on nothing.
#[cfg(test)]
macro_rules! refuses_to_observe {
    ($fake:ty) => {
        impl $crate::operation::machine::Observed for $fake {
            fn read_processes(
                &mut self,
            ) -> Result<
                $crate::operation::machine::ProcessRead,
                $crate::operation::machine::MachineError,
            > {
                Err($crate::operation::machine::MachineError::Refused(
                    concat!(
                        stringify!($fake),
                        " is a test stand-in that observes nothing; an operation that reached \
                         this port has changed"
                    )
                    .to_string(),
                ))
            }
            fn read_memory(
                &mut self,
            ) -> Result<
                $crate::operation::machine::MemoryRead,
                $crate::operation::machine::MachineError,
            > {
                Err($crate::operation::machine::MachineError::Refused(
                    concat!(
                        stringify!($fake),
                        " is a test stand-in that observes nothing; an operation that reached \
                         this port has changed"
                    )
                    .to_string(),
                ))
            }
            fn read_desired_state(
                &mut self,
            ) -> Result<Option<String>, $crate::operation::machine::MachineError> {
                // **Refuses rather than answering `Ok(None)`.** "There is no
                // document" is a CLAIM ABOUT THE MACHINE, and a fake that
                // observes nothing is not entitled to make it — that is the
                // stub-that-pretends `AGENTS.md` §3 refuses.
                Err($crate::operation::machine::MachineError::Refused(
                    concat!(
                        stringify!($fake),
                        " is a test stand-in that observes nothing; an operation that reached \
                         this port has changed"
                    )
                    .to_string(),
                ))
            }
            fn read_journal(
                &mut self,
            ) -> Result<
                $crate::operation::machine::JournalRead,
                $crate::operation::machine::MachineError,
            > {
                Err($crate::operation::machine::MachineError::Refused(
                    concat!(
                        stringify!($fake),
                        " is a test stand-in that observes nothing; an operation that reached \
                         this port has changed"
                    )
                    .to_string(),
                ))
            }

            /// **Refuses rather than answering with unset flags.** A
            /// `ThermalRead` whose `throttle` was `Some(Throttle::default())`
            /// would assert that this machine has never been under-volted,
            /// which is exactly the claim `ThermalRead::throttle_unmeasured`
            /// exists to stop anything making without a measurement.
            fn read_thermal(
                &mut self,
            ) -> Result<
                $crate::operation::machine::ThermalRead,
                $crate::operation::machine::MachineError,
            > {
                self.read_journal().map(|_| unreachable!())
            }

            /// **Refuses rather than answering an idle machine.** A
            /// `CpuRead` of zeroed counters is what a machine that has run
            /// nothing since it booted looks like, and a load average of zero
            /// is what an idle one looks like — both are readings, and a
            /// stand-in that observed nothing has taken neither.
            fn read_cpu(
                &mut self,
            ) -> Result<
                $crate::operation::machine::CpuRead,
                $crate::operation::machine::MachineError,
            > {
                self.read_journal().map(|_| unreachable!())
            }

            /// **Refuses rather than answering an uptime of zero.** `0` would
            /// say this machine booted a moment ago, which is a reading nobody
            /// took.
            fn read_boot(
                &mut self,
            ) -> Result<
                $crate::operation::machine::BootRead,
                $crate::operation::machine::MachineError,
            > {
                self.read_journal().map(|_| unreachable!())
            }

            /// **Refuses rather than answering that this machine trusts
            /// nobody.** An empty trust store is a real reading — it is what
            /// `ADR-028` §5.5.5's `--no-trust` image produces — and a stand-in
            /// that observed nothing has not taken it. The direction is the
            /// dangerous one: *"trusts nobody"* is the answer somebody acts on
            /// by re-signing an artifact.
            fn read_trust(
                &mut self,
            ) -> Result<
                $crate::operation::machine::TrustRead,
                $crate::operation::machine::MachineError,
            > {
                self.read_journal().map(|_| unreachable!())
            }

            /// **Refuses rather than naming a slot.** Naming the wrong one is
            /// how an update overwrites the running system, so a stand-in that
            /// observes nothing must not name any.
            fn read_slots(
                &mut self,
            ) -> Result<
                $crate::operation::machine::SlotRead,
                $crate::operation::machine::MachineError,
            > {
                self.read_journal().map(|_| unreachable!())
            }

            /// **Refuses rather than answering an empty network.** No
            /// interfaces at all reads as a machine with none, which is not a
            /// state a running Linux is in.
            fn read_network(
                &mut self,
            ) -> Result<
                $crate::operation::machine::NetworkRead,
                $crate::operation::machine::MachineError,
            > {
                self.read_journal().map(|_| unreachable!())
            }

            /// **Refuses rather than answering that this machine has no block
            /// devices**, which is not a state a machine that booted off one
            /// can be in.
            fn read_devices(
                &mut self,
            ) -> Result<
                $crate::operation::machine::DeviceRead,
                $crate::operation::machine::MachineError,
            > {
                self.read_journal().map(|_| unreachable!())
            }

            fn configuration_schema(&mut self) -> $crate::value::Value {
                $crate::value::Value::str(concat!(
                    "<no schema: ",
                    stringify!($fake),
                    " is a test fake>"
                ))
            }

            fn supervisor_status(
                &mut self,
            ) -> Result<
                $crate::operation::status::Status,
                $crate::operation::machine::MachineError,
            > {
                self.read_journal().map(|_| unreachable!())
            }

            fn read_filesystems(
                &mut self,
            ) -> Result<
                $crate::operation::machine::StorageRead,
                $crate::operation::machine::MachineError,
            > {
                self.read_journal().map(|_| unreachable!())
            }
        }
    };
}

#[cfg(test)]
pub(crate) use refuses_to_observe;

/// **A [`Machine`] that refuses everything**, completing the set with
/// [`refuses_to_edit`], [`refuses_to_deploy`] and [`refuses_to_observe`].
///
/// `crate::operation::edit`'s fake is an [`Editor`] and nothing else, and since
/// 2026-08-26 it has to be a whole [`Board`] to reach `super::plan::resolve`.
/// It refuses here rather than pretending: an edit test that started arming a
/// trial boot or stopping the machine fails loudly (`AGENTS.md` §3).
#[cfg(test)]
macro_rules! refuses_to_manage {
    ($fake:ty) => {
        impl $crate::operation::machine::Machine for $fake {
            fn running_slot(
                &mut self,
            ) -> Result<$crate::state::Slot, $crate::operation::machine::MachineError> {
                Err($crate::operation::machine::MachineError::Refused(
                    concat!(
                        stringify!($fake),
                        " is a test stand-in that manages no machine; an operation that reached \
                         this port has changed"
                    )
                    .to_string(),
                ))
            }

            fn can_arm_trial_boot(
                &mut self,
                _slot: $crate::state::Slot,
            ) -> Result<(), $crate::operation::machine::MachineError> {
                self.running_slot().map(|_| ())
            }

            fn arm_trial_boot(
                &mut self,
                _slot: $crate::state::Slot,
            ) -> Result<
                $crate::operation::machine::Reboot,
                $crate::operation::machine::MachineError,
            > {
                self.running_slot().map(|_| unreachable!())
            }

            fn can_request_stop(
                &mut self,
                _how: $crate::operation::machine::Stop,
            ) -> Result<(), $crate::operation::machine::MachineError> {
                self.running_slot().map(|_| ())
            }

            fn request_stop(
                &mut self,
                _how: $crate::operation::machine::Stop,
            ) -> Result<(), $crate::operation::machine::MachineError> {
                self.running_slot().map(|_| ())
            }

            fn can_request_reload(
                &mut self,
            ) -> Result<(), $crate::operation::machine::MachineError> {
                self.running_slot().map(|_| ())
            }

            fn request_reload(
                &mut self,
            ) -> Result<(), $crate::operation::machine::MachineError> {
                self.running_slot().map(|_| ())
            }

            fn can_send_input(
                &mut self,
            ) -> Result<(), $crate::operation::machine::MachineError> {
                self.running_slot().map(|_| ())
            }

            fn send_input(
                &mut self,
                _action: &str,
            ) -> Result<(), $crate::operation::machine::MachineError> {
                self.running_slot().map(|_| ())
            }

            fn can_request_service_restart(
                &mut self,
                _service: &str,
            ) -> Result<
                $crate::operation::service_restart::Restarting,
                $crate::operation::machine::MachineError,
            > {
                self.running_slot().map(|_| unreachable!())
            }

            fn request_service_restart(
                &mut self,
                _service: &str,
            ) -> Result<
                $crate::operation::service_restart::Restarting,
                $crate::operation::machine::MachineError,
            > {
                self.running_slot().map(|_| unreachable!())
            }
        }
    };
}

#[cfg(test)]
pub(crate) use refuses_to_manage;

/// What an implementor of [`Editor::read_file`] found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileRead {
    /// The bytes, or `None` when there is no such file.
    ///
    /// **`None` is not an empty file**, and the two must not be confused: an
    /// empty file is one `--expect` can be refused against, and an absent one is
    /// a creation.
    pub contents: Option<Vec<u8>>,
    /// The parent directories of the path that do not exist, **outermost
    /// first**.
    ///
    /// Empty when everything on the way to the path is already there, which is
    /// the ordinary case. Order is the contract: it is the order they would be
    /// created in, and it is what the plan names so that a creation reads as
    /// intent rather than as a silent `mkdir -p` (`ADR-006` §8.9).
    pub missing_parents: Vec<String>,
}

/// One window of a file's bytes, as [`Editor::read_file_window`] found them.
///
/// **A struct rather than two return values**, because the two travel
/// together: `bytes` alone cannot say whether it is the whole rest of the
/// file, and `total_bytes` alone carries no content.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileWindow {
    /// The bytes actually read — at most the `len` asked for, and fewer at
    /// the end of the file.
    pub bytes: Vec<u8>,
    /// The file's size, in bytes, as this same read observed it — not a
    /// stat taken earlier and carried along. See [`Editor::read_file_window`].
    pub total_bytes: usize,
}

/// One whole-file replacement, as [`Editor::write_file`] is asked for it.
///
/// **A struct rather than five parameters**, because four of the five are what
/// the *record* has to say and passing them positionally is how a journal comes
/// to describe the wrong edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Write<'a> {
    /// The file to replace.
    pub path: &'a str,
    /// Its new contents, whole. Text, because the domain has already refused a
    /// file that is not UTF-8.
    pub contents: &'a str,
    /// Directories to create first, **outermost first** — exactly
    /// [`FileRead::missing_parents`], carried through so the write creates what
    /// the plan named and nothing else.
    pub directories: &'a [String],
    /// Whether this path had no file before. Decides the mode an implementor
    /// creates with (`crate::operation::edit::NEW_FILE_MODE`) and is what the
    /// record says.
    pub creating: bool,
    /// How the new contents were arrived at, in the operation's own word —
    /// `replace` or `append`. For the record, never for the write.
    pub how: &'static str,
    /// How many bytes were there before. For the record.
    pub was: u64,
}

/// What an implementor of [`Editor::write_file`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Wrote {
    /// Why this edit is **not** in this machine's journal, or `None` when it
    /// is.
    ///
    /// **Not a refusal**, and [`Editor::write_file`] carries the argument: the
    /// file is already replaced, and saying *nothing was written* about it would
    /// be worse than saying *it happened and is unrecorded*. The operation
    /// reports both to the operator rather than choosing for them.
    pub unrecorded: Option<String>,
}

/// What a machine can be **asked**, as opposed to told.
///
/// Split out of [`Machine`] on 2026-08-24, at the point that trait's own
/// documentation named: *"a `Machine` that grows a sixth method belonging to
/// neither is the signal to cut it."* The signal fired twice — `read_journal`
/// was the sixth and `read_filesystems` the seventh — and the comment had
/// already worked out both the name and the membership, so this is the cut it
/// specified rather than a new opinion.
///
/// **The line is authority, not subject matter.** Everything on [`Machine`]
/// changes the machine and comes in a `can_`/do pair, because `ADR-006` §7's
/// plan-then-apply needs to ask before acting. Nothing here has a `can_` half,
/// because there is nothing to refuse: reading is always allowed and always
/// answerable. That is why the two moved together and why nothing else moved
/// with them.
///
/// **What the cut is actually worth**, since the trait was right that it had a
/// cost. Before it, every test fake implemented all seven methods — a fake
/// journal had to stub `read_filesystems` and a fake storage had to stub
/// `read_journal`, so adding a method to the port edited every unrelated test.
/// Each operation now names the port it needs, which is `ADR-014` §3's rule
/// applied one level in: `get log` and `get storage` take `&mut dyn Observed`
/// and cannot reboot the machine if they try.
///
/// **`perform` takes two ports rather than one, and two is not five.** The
/// objection recorded on [`Machine`] was that a composition root assembling
/// *five* ports would make `ADR-006` §5.1's network adapter expensive in the
/// way §3 forbids. That objection is about a count, and it is answered by
/// stopping at the seam the authority actually has. A future read-only fact —
/// `ADR-028` §5.5.6's `get trust`, for one — lands **here**, on a port that
/// already exists, rather than becoming a third parameter.
pub trait Observed {
    /// Every journal line this machine has kept, **oldest first**.
    ///
    /// `ADR-022` §9's query, and the second thing on this port that reads the
    /// world rather than changing it.
    ///
    /// # Why it returns *lines* and not records
    ///
    /// Because `ADR-011`. `ADR-022` §3 makes the JSONL form authoritative and
    /// [`crate::journal::Record::from_jsonl`] is the one reader of it — so an
    /// adapter that returned parsed records would be a **second** reader, and
    /// the two could disagree about a line without either of them failing.
    /// What an adapter knows that the domain cannot is *which files*, in what
    /// order, on a filesystem; what the domain knows is what a line means.
    /// The seam goes between those two.
    ///
    /// It also keeps the port honest about `ADR-014` §3: a `Vec<String>` names
    /// nothing this crate is forbidden to name, which is `AGENTS.md` §3.1's
    /// test for a domain port.
    ///
    /// # Order is the contract, and it is load-bearing
    ///
    /// `crate::journal::boots_in_order` takes the sequence as the authority for
    /// which boot came first, because a boot identifier is a random UUID and a
    /// machine with no RTC starts every boot at the epoch (`ADR-021`). An
    /// implementor that concatenated its files newest-first would make
    /// `--boot previous` name the wrong boot — an answer that looks right.
    ///
    /// # An empty journal is `Ok`, not an error
    ///
    /// A machine that has never written a record, and a machine whose `/state`
    /// never mounted, both have nothing to show — and *"there are no records"*
    /// is a true answer. What must not happen is that being confused with *"the
    /// journal could not be read"*, which is what [`JournalRead::files`] is for.
    fn read_journal(&mut self) -> Result<JournalRead, MachineError>;

    /// Every filesystem this machine currently has mounted, and how much of each
    /// is in use.
    ///
    /// `ADR-005` §4.1.3's `df` row, and the third thing on this port that reads
    /// the world rather than changing it. **It matters now rather than
    /// eventually**: `ADR-018` §10 is accepted and unimplemented, so `/state`
    /// does not grow to fill its partition, and `ADR-022` §8 caps the journal at
    /// 8 MiB against a card whose free space nothing on the machine could
    /// measure.
    ///
    /// # Why it returns *block counts* and not bytes
    ///
    /// The same seam [`Self::read_journal`] draws, for the same reason. What an
    /// adapter knows that the domain cannot is *which filesystems, from which
    /// table, through which syscall*; what the domain knows is **what "used"
    /// means** — and that is a real judgement rather than a subtraction:
    ///
    /// > A filesystem's used space is `blocks - blocks_free`, and the space a
    /// > caller may still have is `blocks_available`. Those two do **not** add
    /// > up to `blocks`, because a filesystem reserves blocks that are neither
    /// > free to an ordinary caller nor in use. Computing `used` as
    /// > `total - available` therefore over-reports it by the reserve, and on a
    /// > default `ext4` the reserve is 5% of the partition.
    ///
    /// `df` gets this right and code written from first principles usually does
    /// not. Putting the arithmetic in the domain makes it one implementation
    /// serving the nushell table, the future network API and a dashboard
    /// (`ADR-005` §4.1.1), and — the part that decides it — makes it **testable
    /// with no I/O at all**, which `ADR-014` §3.1 requires of this crate.
    ///
    /// It also keeps the port honest about `ADR-014` §3: nothing in
    /// [`MountedFilesystem`] is a type this crate is forbidden to name. There is
    /// no `statvfs`, no `Errno`, no mount flag and no device node — only the
    /// strings the machine reported and the counts it gave, which is
    /// `AGENTS.md` §3.1's test for a domain port.
    ///
    /// # A filesystem that could not be measured is reported, never dropped
    ///
    /// [`StorageRead::unreadable`] exists for the same reason `get log` counts
    /// the lines it could not parse: an answer that omits a filesystem says
    /// *there is no such filesystem*, which is the one thing this operation must
    /// not say falsely (`AGENTS.md` §5). An implementor that cannot measure one
    /// entry of the mount table records it there and carries on.
    ///
    /// # An empty answer is `Ok`, and a machine that has no mount table is not
    ///
    /// A machine with no filesystems mounted is not a state Linux can be in, so
    /// an empty [`StorageRead::filesystems`] with an empty
    /// [`StorageRead::unreadable`] means the *table* was empty — which is why
    /// [`StorageRead::mount_table`] names where it was read from. Being unable
    /// to read the table at all is a [`MachineError::Refused`] carrying what the
    /// system said, never an empty list.
    fn read_filesystems(&mut self) -> Result<StorageRead, MachineError>;

    /// **The configuration document this machine will accept.** `ADR-006` §7's
    /// self-description, as a subject of `get` since 2026-08-25.
    ///
    /// # Why it is on a port at all, rather than computed here
    ///
    /// `ADR-014` §3.2 makes `rivet-config` depend on `rivet-core`
    /// **permanently**, so `Document` and its schema are not things this crate
    /// may name — the same edge that decided `explain` would be the first
    /// operation (`ADR-006` §8.2). The adapter has both crates and hands the
    /// value over; the catalogue entry, the subject and the refusals stay here.
    ///
    /// # Why it returns no `Result`
    ///
    /// Nothing can fail. The other two methods read a running machine and can
    /// be refused by it; this is a value the build computes from its own types.
    /// A `Result` here would be a fault that cannot occur, and every caller
    /// would have to write a branch for it that could never be tested — which
    /// is the shape `AGENTS.md` §3 calls a stub that looks further along than
    /// it is.
    ///
    /// # Why it is `get` and not a bare word — ruled 2026-08-25
    ///
    /// It was `rivet schema`, beside `--help` and `--version`, on the reading
    /// that §6.4's verbs are *things done to a machine* while this is *the
    /// surface describing itself*. Two things killed that:
    ///
    /// 1. **`rivet schema` has no verb in it.** The maintainer's ruling
    ///    (`.rulings/` R-017): `measure` is a verb that describes what the tool
    ///    is doing, and `schema` was not. `measure hash` reads as
    ///    verb-then-object and stays a bare word; a bare noun as a whole
    ///    command does not.
    /// 2. **It was undiscoverable.** `rivet --version` publishes the
    ///    catalogue and a board answered `apply, explain, get` — so a machine
    ///    reading the catalogue to find out what it could do would never learn
    ///    `schema` existed. `ADR-012` makes an agent a first-class operator;
    ///    a surface with invisible exceptions cannot be driven from its own
    ///    description.
    ///
    /// The defence also broke on `explain`, which performs no I/O and answers
    /// identically on every machine of a build — so *"describes the build
    /// rather than the machine"* cannot be what separates a verb from a bare
    /// word, because `explain` has that property and is a verb.
    ///
    /// **And on a fleet it really is a per-machine fact.** A/B slots and
    /// staggered updates mean two boards can run different builds and accept
    /// different documents. *Ask this machine what it accepts* is the useful
    /// question; *what does the tool describe* stops being one the moment there
    /// is more than one machine.
    /// **The desired-state document, as bytes on disk.** `ADR-035` phase 4.
    ///
    /// `Ok(None)` when there is none, which is an ordinary answer rather than a
    /// fault: a machine boots from the provisioning document on the boot
    /// partition and writes `/state/config` only when it has something to write
    /// (`ADR-004` §6). An error means it is there and could not be read.
    ///
    /// **It takes no path**, and that is the point. `crate::operation::edit`'s
    /// port reads an arbitrary file because editing one is its job; this reads
    /// the ONE file this build knows the name of, so it needs none of the
    /// decisions a general reader owes — about `/state/secrets`, about device
    /// nodes, about a directory, about how much of a large file to return.
    fn read_desired_state(&mut self) -> Result<Option<String>, MachineError>;

    /// **What this machine's memory looks like right now.** `ADR-035` phase 4.
    ///
    /// An error means the kernel's own files could not be read, which on a
    /// running Linux is a fault worth reporting rather than an empty answer.
    /// A kernel with no PSI is NOT an error — see [`MemoryRead::pressure`].
    fn read_memory(&mut self) -> Result<MemoryRead, MachineError>;

    /// **Every process, read from `/proc`.** `ADR-035` phase 4.
    ///
    /// This is NOT [`Self::supervisor_status`]. That answers *what services
    /// does the supervisor believe it is running*, from its own memory; this
    /// answers *what is actually executing*, from the kernel. `ADR-011` §3 is
    /// the reason both exist: two things that both look authoritative, and the
    /// interesting cases are exactly where they disagree.
    fn read_processes(&mut self) -> Result<ProcessRead, MachineError>;

    /// **What this machine's processors have spent their time on.** `ADR-035`
    /// phase 4, and the fact the monitor pane could not show at all until
    /// 2026-08-29.
    ///
    /// # It answers counters, and refuses to answer a percentage
    ///
    /// [`CpuRead`] carries the whole argument. In one line: **utilisation is a
    /// rate**, a rate needs two readings, and this port takes one. A method that
    /// returned a percentage would have to invent the second reading or the
    /// interval, and a plausible number nobody measured is `AGENTS.md` §5's
    /// guess that succeeds — the worst kind, because it looks exactly like the
    /// thing working.
    ///
    /// The load average is the exception and is included for that reason: the
    /// **kernel** maintains it across time, so a single read of it is a real
    /// measurement rather than a first sample.
    ///
    /// An error means the kernel's own files could not be read, which on a
    /// running Linux is a fault worth reporting rather than an empty answer. A
    /// machine that states no load average is NOT an error — see
    /// [`CpuRead::load`].
    fn read_cpu(&mut self) -> Result<CpuRead, MachineError>;

    /// **Which signing keys this machine accepts.** `ADR-028` §5.5.6, which is
    /// the ruling this port exists to satisfy, in that section's own words:
    ///
    /// > Security is not the discriminator … What decides it is that **a machine
    /// > should be able to say what it trusts.** `rivet get trust` returning the
    /// > labels, key ids and masks is a machine fact in `ADR-006` §3's sense; a
    /// > constant in a stripped binary is a fact nobody can ask for.
    ///
    /// Built 2026-08-29, Pile B item 3 (`PLAN.md` §102.3 item 2). The store has
    /// existed since 2026-08-24 and nothing could read it back except the two
    /// binaries about to verify something with it — so *"will this board install
    /// what I signed"* was answerable only by trying.
    ///
    /// # An empty store is an ANSWER; an unreadable one is a refusal
    ///
    /// The distinction is the whole of `AGENTS.md` §5 here, and
    /// `cmd/rivet-deploy` already draws it the same way for the same file:
    ///
    /// | On the machine | This port |
    /// | :-- | :-- |
    /// | a store with no entries — what `--no-trust` builds (`ADR-028` §5.5.5) | **`Ok`** with no keys. The machine really does trust nobody, and that is a reading |
    /// | no store at all, or one that will not parse | **`Err`**. *"A store that cannot be read is not a store that trusts nobody"*, and treating it as one is how a corrupt file becomes an install nobody authorised |
    ///
    /// # It is read at the point of use and never cached
    ///
    /// `AGENTS.md` §5: the kernel — here, the filesystem — is the authority.
    /// `ADR-028` §5.5.4 makes revocation an A/B image update that ships a store
    /// without the key, so a store remembered from start-up would answer with
    /// the one the machine booted under. `cmd/rivet-sysd`'s launch check already
    /// re-reads it on every `execve` for exactly that reason.
    fn read_trust(&mut self) -> Result<TrustRead, MachineError>;

    /// **What this machine's silicon is doing to itself.** `ADR-035` phase 4,
    /// and `docs/design-candidates/Admin-Surface-Survey.md` §7.
    ///
    /// # The name is `thermal` and the subject is deliberately not `temperature`
    ///
    /// §7.1 corrected the framing before anything was built: *"temperature is
    /// the least useful part of thermals on a Pi."* 70 °C is fine, and a board
    /// that throttled an hour ago reads cool by the time anyone looks. The four
    /// facts, weakest first, are the temperature, the clock against its
    /// ceiling, the firmware's throttle flags now, and **the sticky
    /// since-boot half of those flags** — which is the only one that cannot be
    /// recovered any other way.
    ///
    /// # It must not be Pi-shaped
    ///
    /// §7.2: Rivet's long-term target includes an x86 machine with no Broadcom
    /// mailbox at all. So this port names *zones*, *a frequency against a
    /// maximum* and *a throttle state* — and the Pi's firmware bitmask and
    /// x86's `coretemp` under `/sys/class/hwmon` are both **adapters** to it.
    /// Nothing here is a bitmask, a mailbox tag or a `/dev` node, which is
    /// `AGENTS.md` §3.1's test for a domain port.
    ///
    /// # Flags that were not read are reported as unread, never as false
    ///
    /// [`ThermalRead::throttle_unmeasured`] carries the rule, and it is
    /// [`MemoryRead::pressure`]'s: `false` would assert *this board has never
    /// been throttled*, which is the single most valuable claim this subject can
    /// make and the one it must never make without a measurement.
    fn read_thermal(&mut self) -> Result<ThermalRead, MachineError>;

    /// **Which boot this is, and how long it has been going.** `ADR-035` phase
    /// 4, and `docs/design-candidates/Admin-Surface-Survey.md` §4.
    ///
    /// # Why the uptime is the trustworthy half and the clock is not
    ///
    /// `ADR-021`: **a Raspberry Pi has no real-time clock.** Every boot starts
    /// at the epoch until something corrects it, and when something does, the
    /// wall clock **jumps**. So a duration measured from the monotonic counter
    /// is a fact and a timestamp is a claim, and every timestamp reading on this
    /// machine depends on knowing which of the two you are holding.
    ///
    /// That is why [`BootRead::uptime_seconds`] is not optional and why nothing
    /// here is a wall-clock time. The seconds since boot are the same number
    /// whether or not the clock was ever set.
    ///
    /// # The list of boots comes from the journal, and it is not this port's
    ///
    /// [`crate::journal::boots_in_order`] already computes it, from
    /// [`Self::read_journal`], and a second implementation would be `ADR-011`
    /// §3 exactly: two answers to *"which boot came before this one"*, and the
    /// disagreement would surface as `--boot previous` naming a different boot
    /// from the one this subject reported. So this method answers only what the
    /// kernel says about **this** boot, and the operation composes the two.
    fn read_boot(&mut self) -> Result<BootRead, MachineError>;

    /// **Which A/B slot this machine is running, and what is armed.**
    /// `ADR-035` phase 4, `PLAN.md` §92.3, and
    /// `docs/design-candidates/Admin-Surface-Survey.md` §4, which says why it
    /// earns a subject: *"reconstructed by hand from `/proc/cmdline` twice this
    /// week."*
    ///
    /// # Two authorities, deliberately, and the disagreement is the finding
    ///
    /// [`Machine::running_slot`] already answers *which boot partition did the
    /// firmware read* — from the device tree, which is the firmware's own
    /// statement. That is **not the same question** as *which system partition
    /// did the kernel mount as `/`*, which comes from
    /// [`SlotRead::root_argument`].
    ///
    /// They should agree, because `boot-a`'s `cmdline.txt` names `system-a`
    /// (`ADR-018` §4). When they do not, this machine booted one slot's
    /// firmware configuration into the other slot's filesystem — and arming a
    /// trial boot on it would write the wrong partition. So both are reported
    /// and neither is folded into the other, which is `super::process`'s
    /// arrangement for the same reason: `ADR-011` §3 usually means deleting one
    /// of two authorities, and here the interesting case is exactly where they
    /// differ.
    ///
    /// # Never from a binary's hash
    ///
    /// The running slot is read from the kernel command line or the mount
    /// table. Comparing a hash of the running binary against each slot's copy
    /// looks like it would work and **fails silently** — two slots holding the
    /// same build produce the same hash, so the answer is a coin toss that
    /// looks like a reading. That has cost this project time already.
    fn read_slots(&mut self) -> Result<SlotRead, MachineError>;

    /// **This machine's interfaces, addresses, routes and resolvers.**
    /// `ADR-035` phase 4, and `docs/design-candidates/Admin-Surface-Survey.md`
    /// §3.1 and §7.4.
    ///
    /// # `rivet-netd` owns the wire and answers nothing
    ///
    /// `rivet-adapter-net` brings interfaces up over netlink and assigns
    /// addresses; **nothing in this system could report what it did.** A board
    /// whose network is wrong has been debugged by logging in over the network
    /// that is wrong, which is the circularity this method exists to break.
    ///
    /// # The seam, in this method's terms
    ///
    /// An implementor knows *which files, in what format, on which
    /// filesystem* — `/proc/net/*` and `/sys/class/net/*`, hexadecimal, little
    /// endian, one directory per interface. The domain knows **what the numbers
    /// mean**: that an operational state of `down` with a route through it is a
    /// contradiction, and that error counters climbing on one interface is a
    /// cable rather than a configuration. Nothing in [`NetworkRead`] is a
    /// libc type, an `Errno`, a socket or a netlink message
    /// (`AGENTS.md` §3.1).
    ///
    /// # What was not read is listed, not omitted
    ///
    /// [`NetworkRead::unmeasured`] is the honest half. This surface reads what
    /// `/proc` and `/sys` expose, and there are facts they do not — IPv4
    /// addresses are not in either, and are reachable only over netlink. An
    /// answer that simply had no IPv4 addresses in it would say this machine
    /// has none, which is exactly the omission `AGENTS.md` §5 forbids on a
    /// machine you reached over IPv4.
    fn read_network(&mut self) -> Result<NetworkRead, MachineError>;

    /// **Every block device this machine has, and what each one has carried.**
    /// `ADR-035` phase 4, and `docs/design-candidates/Admin-Surface-Survey.md`
    /// §7.3.
    ///
    /// # It is not `get storage`, and the difference is which question
    ///
    /// | | |
    /// | :-- | :-- |
    /// | `get storage` | how full each **mounted filesystem** is — `ADR-005` §4.1.3 |
    /// | `get device` | what **hardware** is there, whether or not anything is mounted on it |
    ///
    /// A partition that failed to mount is invisible to the first and present
    /// in the second, and on an A/B machine the inactive slot's `/system` is
    /// exactly that: a real partition with no filesystem view.
    ///
    /// # The number this must refuse to invent
    ///
    /// Survey §7.3, on a request for *"wear-levelling metrics and remaining
    /// endurance"*: **an eMMC reports it and a consumer SD card generally does
    /// not.** A subject that produced *"remaining endurance"* for an SD card
    /// would be inventing a number, which is the failure this codebase refuses
    /// everywhere else.
    ///
    /// So [`BlockDevice::life_time`] and [`BlockDevice::pre_eol_info`] are
    /// carried **verbatim where the device states them and absent where it does
    /// not**, and nothing here computes a percentage or a remaining life. What
    /// *is* always available is [`BlockDevice::io`] — bytes actually written —
    /// and `ADR-018` §7 already treats SD wear as a design constraint, so
    /// measuring the write volume closes the loop on decisions taken blind.
    fn read_devices(&mut self) -> Result<DeviceRead, MachineError>;

    fn configuration_schema(&mut self) -> Value;

    /// **What the supervisor is doing, right now.** `ADR-032` §3.2's query
    /// socket, added 2026-08-26.
    ///
    /// **The only method on this port that cannot be answered from a file.**
    /// `read_journal` reads one, `read_filesystems` reads the kernel's table and
    /// `configuration_schema` is computed; this one is a question for another
    /// PROCESS, and the answer exists only in that process's memory
    /// (`AGENTS.md` §5 — derive, never store, which is why no file holds it).
    ///
    /// **A supervisor that is not answering is the ordinary case**, not an
    /// exceptional one: `ADR-017` §3.1 permits it to be dead, and a machine
    /// whose supervisor is down is exactly when somebody asks. `ADR-032` §3.3
    /// item 5 rules the refusal — it names the socket and says the supervisor
    /// is not answering, which is a true and useful statement rather than a
    /// generic I/O error.
    fn supervisor_status(&mut self) -> Result<crate::operation::status::Status, MachineError>;
}

/// What an implementor of [`Observed::read_journal`] found.
///
/// **`files` is not decoration.** `ADR-022` §8 rotates and drops the oldest, so
/// the set of files that exist is what the journal actually still holds — and a
/// reader that reported only records could not tell "this machine logged
/// nothing" from "this machine's journal is not where I looked".
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JournalRead {
    /// Every line, oldest first, exactly as the machine wrote them. Parsing is
    /// [`crate::journal::parse_lines`]'s.
    pub lines: Vec<String>,
    /// The files that were there and were read, oldest first.
    pub files: Vec<String>,
    /// Where the implementor looked, whether or not it found anything. Reported
    /// so that an empty answer names a place rather than being a shrug.
    pub source: String,
}

/// Every process this machine is running — `ADR-035` phase 4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessRead {
    /// **In ascending pid order**, so two readings of an unchanged machine
    /// produce the same bytes. `/proc` is a directory and its order is the
    /// filesystem's, which is not an order anybody chose.
    pub processes: Vec<ProcessInfo>,
    /// Processes that were there when the directory was listed and gone by the
    /// time they were read.
    ///
    /// **This is the ordinary case, not a fault.** `/proc` is a snapshot that
    /// changes while it is being read, and a short-lived process disappearing
    /// mid-read is the system working. It is reported rather than dropped for
    /// [`StorageRead::unreadable`]'s reason: a count that silently shrinks is a
    /// count nobody can reason about.
    pub vanished: u32,
}

/// One process, from `/proc/<pid>/status`.
///
/// **One file, not five.** `status` carries the name, state, parent, thread
/// count and resident size together, so a process that exits mid-read is either
/// wholly read or wholly missed — never half of one process and half of the
/// state it was in a moment later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub parent_pid: u32,
    /// **Truncated to 15 characters by the kernel**, which is a property of
    /// `comm` and not of this build. A longer program name is not recoverable
    /// from here.
    pub name: String,
    /// `R` running, `S` sleeping, `D` uninterruptible, `Z` zombie, `T` stopped.
    ///
    /// **`D` and `Z` are the two worth looking for.** Uninterruptible sleep is
    /// a process stuck in the kernel — on a Pi, usually on I/O to a failing SD
    /// card — and it cannot be killed. A zombie means its parent is not reaping,
    /// which on Rivet means the supervisor is not doing its job.
    pub state: String,
    pub threads: u32,
    /// Resident set size. **`None` for a kernel thread**, which has no user
    /// address space and therefore no `VmRSS` line — that absence is what
    /// [`Self::kernel`] is read from, rather than from the name's brackets.
    pub resident_bytes: Option<u64>,
    /// Whether this is a kernel thread.
    ///
    /// Worth reporting because on a Rivet the *userland* list should be short
    /// and entirely accounted for by the supervisor. A userland process that is
    /// not one of its services is an anomaly, and it cannot be spotted without
    /// separating the two.
    pub kernel: bool,
    /// CPU time this process has spent in **user mode**, in milliseconds since
    /// it started. Added 2026-08-29.
    ///
    /// # Why it is here, and what it is not
    ///
    /// **Without it there is no per-process CPU figure at all**, however many
    /// times a caller reads this table: `pid`, `name`, `state`, `threads` and
    /// `resident_bytes` are all *states*, and none of them accumulates. Two
    /// readings of a state are two states. Two readings of a counter are a rate,
    /// and this is the counter.
    ///
    /// **It is still not a percentage**, and a caller holding one reading of it
    /// knows nothing about how busy this process is — [`CpuRead`] carries the
    /// same argument for the machine as a whole.
    ///
    /// **`None` where it could not be read**, never `0`: zero milliseconds of
    /// CPU is a real reading of a process that has done nothing, and it is a
    /// meaningfully different claim from *this build did not look*. The two
    /// figures come from a different file in `/proc` than the rest of this
    /// struct, so they can go missing on their own — see `crate::process` in
    /// `cmd/rivet` for what that costs.
    pub user_millis: Option<u64>,
    /// CPU time spent in **kernel mode** on this process's behalf, in
    /// milliseconds. [`Self::user_millis`]'s rules, and worth keeping separate:
    /// a process that is all system time is one making syscalls rather than one
    /// computing, which is a different problem.
    pub system_millis: Option<u64>,
    /// **The kernel function this process is parked in**, from
    /// `/proc/<pid>/wchan`. Added 2026-09-16.
    ///
    /// # Why a counter was not enough
    ///
    /// [`Self::user_millis`] and [`Self::system_millis`] say *whether* a process
    /// is doing anything. They cannot say **what it is waiting for**, and on
    /// 2026-09-16 that was the whole question: `rivet-sysd` stopped with every
    /// counter frozen, `state` reading `S`, and two live hypotheses — blocked
    /// writing an answer nobody was reading, or blocked in a filesystem read —
    /// that a frozen counter cannot tell apart. They are **one word apart here**:
    /// `unix_stream_read_generic` and its write twin against anything in the
    /// block layer. On a healthy supervisor this reads `do_sys_poll`.
    ///
    /// See `docs/design-candidates/Supervisor-Blocking-Audit-2026-09-16.md` §10,
    /// which ruled three of five hypotheses out by evidence and could separate
    /// the last two by nothing it had.
    ///
    /// **`None` where it could not be read**, never an empty string, for
    /// `user_millis`'s reason exactly. The kernel also writes the literal `0`
    /// for a process it will not answer about, and the adapter reports that as
    /// `None` rather than passing a symbol named zero to a reader.
    pub wchan: Option<String>,
    /// **The syscall this process is stopped in**, from `/proc/<pid>/syscall` —
    /// the number first, then its arguments and the stack and program counters.
    /// Added 2026-09-16, with [`Self::wchan`] and for its reason.
    ///
    /// **It is the sharper of the two and the less portable.** `wchan` names a
    /// kernel function; this names the call the program made, which on `aarch64`
    /// is `73` for `ppoll` and `64` for `write` — and a supervisor stopped in
    /// `write` is a different defect from one stopped in `openat`, with no
    /// further reading needed to say which.
    ///
    /// **`None` for a kernel thread**, which made no syscall — though the
    /// kernel says that by writing `0 0x0 0x0 …` rather than by writing
    /// nothing, and the adapter turns that line into the absence it means.
    /// Reporting it would be worse than noise: syscall `0` on `aarch64` is
    /// `io_setup`, so every kernel thread would appear stopped in one.
    ///
    /// `None` too where the file is absent or refused. But the kernel writes
    /// `running` for a task that is on a CPU rather than parked, and **that is
    /// reported as-is** — *it is running* is an answer.
    pub syscall: Option<String>,
}

/// What a machine says about its memory — `ADR-035` phase 4.
///
/// **Bytes, not kibibytes.** `/proc/meminfo` is in kB and every tool that
/// forwards that unit forces every reader to remember it. The adapter converts
/// once, here, so nothing downstream has to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRead {
    /// Everything the kernel manages.
    pub total_bytes: u64,
    /// **The honest number, and the one to read.** `MemAvailable` is the
    /// kernel's own estimate of what a new allocation could get — free memory
    /// plus what it would reclaim. Reporting [`Self::free_bytes`] alone is the
    /// classic misreading of Linux memory: a healthy machine has almost no free
    /// memory because it is all page cache, and an operator who reads it as
    /// exhaustion will go looking for a problem that is not there.
    pub available_bytes: u64,
    /// Untouched memory. Kept because it is a fact, not because it is useful.
    pub free_bytes: u64,
    /// Page cache — the reason `free` and `available` differ so widely.
    pub cached_bytes: u64,
    pub swap_total_bytes: u64,
    pub swap_free_bytes: u64,
    /// Pressure stall information, when this kernel has it.
    ///
    /// **`None` is a fact about the kernel, not a failure to read.** PSI needs
    /// `CONFIG_PSI` and a kernel at least 4.20, and a build without it has no
    /// `/proc/pressure` at all.
    pub pressure: Option<MemoryPressure>,
    /// What the kernel holds for itself, line by line as it states it.
    pub kernel: KernelMemory,
    /// Every slab cache the kernel listed, when it could be read.
    ///
    /// **`None` is a source that could not be read** — `/proc/slabinfo` is
    /// readable by root only, and a kernel without `CONFIG_SLUB_DEBUG` has no
    /// such file. It is reported as absence, never as a machine with no slab.
    pub slab_caches: Option<SlabCaches>,
}

/// What the kernel's slab cache listing said: every cache, or why it could not
/// be understood.
///
/// **A listing that was read and did not parse is a refusal carrying the text
/// that did not parse**, never a listing with that line left out. A cache
/// skipped silently is the one that would have named the leak.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlabCaches {
    /// Every cache, in the order the kernel listed them.
    Read(Vec<SlabCache>),
    /// The listing was read and is not in a form this build understands.
    Refused(String),
}

/// One slab cache as the kernel's listing states it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlabCache {
    /// The cache's name. Under slab merging this is the name of the first
    /// cache created at this size, and [`Self::merged`] names the others.
    pub name: String,
    /// Objects in use.
    pub active_objects: u64,
    /// Objects allocated, in use or not.
    pub objects: u64,
    /// Bytes per object including the allocator's own metadata, as the
    /// listing states it.
    pub object_size: u64,
    /// Every OTHER cache name the kernel merged into this one, so that their
    /// objects are counted here.
    ///
    /// `None` when the kernel's per-cache directory could not be read or did
    /// not name this cache; an empty list is a reading that nothing was merged.
    pub merged: Option<Vec<String>>,
}

impl SlabCache {
    /// What this cache holds: `objects × object_size`. Page-size independent,
    /// and short of the whole slabs by each slab's unused tail.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.objects.saturating_mul(self.object_size)
    }
}

/// The kernel's own holdings, from the lines of `/proc/meminfo` that account
/// for memory no process owns.
///
/// **This is the reading that attributes a loss nobody's resident size
/// explains.** When `available_bytes` falls while every process holds flat, the
/// memory went to slab, page tables, kernel stacks, shared memory or a
/// contiguous allocator region serving the display — and each of those is a
/// separate line the kernel already writes.
///
/// **Every field is optional, independently**, because which lines a kernel
/// writes depends on how it was configured: `CmaTotal` exists only with
/// `CONFIG_CMA`, and older kernels lack others. A line the kernel did not write
/// is `None`, and `None` is reported as absence — a zero would assert that the
/// kernel measured nothing held, which it did not say.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KernelMemory {
    /// `Slab`: every slab cache, reclaimable and not.
    pub slab_bytes: Option<u64>,
    /// `SReclaimable`: the part of `Slab` the kernel would give back under
    /// pressure — dentries and inodes, mostly.
    pub slab_reclaimable_bytes: Option<u64>,
    /// `SUnreclaim`: the part of `Slab` it would not. A steady rise here with
    /// flat processes is the signature of a kernel-side leak.
    pub slab_unreclaimable_bytes: Option<u64>,
    /// `Shmem`: shared memory and tmpfs, which includes buffers handed between
    /// a compositor and its clients.
    pub shmem_bytes: Option<u64>,
    /// `PageTables`: the page tables of every address space.
    pub page_tables_bytes: Option<u64>,
    /// `KernelStack`: one stack per thread, so it follows the thread count.
    pub kernel_stack_bytes: Option<u64>,
    /// `VmallocUsed`: the kernel's virtually contiguous allocations.
    pub vmalloc_used_bytes: Option<u64>,
    /// `CmaTotal`: the contiguous region reserved for devices such as the GPU.
    pub cma_total_bytes: Option<u64>,
    /// `CmaFree`: what of that region is unallocated.
    pub cma_free_bytes: Option<u64>,
}

/// How long tasks stalled on memory, from `/proc/pressure/memory`.
///
/// **In hundredths of a percent**, because [`crate::value::Value`] has no
/// floating-point variant and a whole-number percent would collapse the
/// difference between *0.4% stalled* and *nothing at all* — which is exactly
/// the range where this metric earns its keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryPressure {
    /// SOME task stalled, averaged over 10 seconds.
    pub some_avg10: u32,
    /// ...over 60 seconds. A machine that is fine now but was struggling shows
    /// here and not in `avg10`.
    pub some_avg60: u32,
    /// EVERY runnable task stalled — the machine did no work at all.
    pub full_avg10: u32,
    pub full_avg60: u32,
}

/// What a machine says about its processors — `ADR-035` phase 4.
///
/// # Every figure here is a COUNTER, and a counter is not a percentage
///
/// This is the whole shape of the type and the reason it looks unlike every
/// other reading on this port. `get memory` answers *how much memory is free*
/// and one reading settles it. **CPU utilisation is a rate**, and a rate is a
/// difference between two readings divided by the time between them — so a
/// single read of `/proc/stat` cannot produce one, however much a caller wants
/// it to.
///
/// The counters are therefore carried as they are: **milliseconds of CPU time
/// spent in each state since boot**, monotonically increasing. A caller that
/// wants a percentage takes two readings and subtracts. A caller that has only
/// one has **no percentage**, and `AGENTS.md` §5 says what it must do about
/// that: report that it does not know, never `0%`. Zero percent is a
/// measurement of an idle machine and it is exactly what a first reading must
/// not claim.
///
/// # The load average is the one figure a single reading *can* answer
///
/// Because the kernel maintains it across time itself, rather than the caller
/// doing so. It is included for that reason and it comes with a caveat that
/// belongs beside it rather than in a manual: **Linux's load average is not a
/// CPU utilisation.** It counts tasks in uninterruptible sleep as well as
/// runnable ones, so on a Pi a load of 4 with an idle CPU is a card that has
/// stopped answering — which `ProcessInfo::state`'s `D` says the same thing
/// about, one process at a time. See [`LoadAverage`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuRead {
    /// Every processor's counters, in the order the machine listed them —
    /// ascending, so two readings of an unchanged machine produce the same
    /// bytes.
    ///
    /// **Per-CPU rather than only the total**, because the failure worth
    /// finding on a small board is one core pinned while three idle: an
    /// aggregate divides that by four and it stops looking like anything.
    pub cpus: Vec<CpuTimes>,
    /// The kernel's own aggregate over every processor. **Not computed here
    /// from [`Self::cpus`]** — it is the line the kernel itself writes, and
    /// summing the parts would be a second answer to the same question
    /// (`ADR-011` §3).
    pub total: CpuTimes,
    /// How many processors [`Self::cpus`] describes. Carried rather than left
    /// to `cpus.len()` so that a caller dividing a busy figure by a core count
    /// is dividing by the number the machine stated.
    pub processors: u32,
    /// The kernel's load average, or `None` where this machine states none.
    ///
    /// **`None`, never three zeroes.** A load average of zero is a real reading
    /// of an idle machine, and it is the reading a caller is most likely to act
    /// on without checking.
    pub load: Option<LoadAverage>,
    /// Present exactly when [`Self::load`] is absent, and says why.
    pub load_unmeasured: Option<String>,
    /// Where the counters were read from, so an answer names a place.
    pub source: String,
}

/// **What a machine will accept a signature from.** [`Observed::read_trust`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustRead {
    /// Where the store was read, so the answer names a place rather than
    /// implying one. `ADR-028` §5.5.6 puts it at a fixed path deliberately —
    /// *"a configurable trust anchor is the thing §5.2 exists to forbid"* — and
    /// reporting it is how an operator confirms that is still where it came
    /// from.
    pub source: String,
    /// **One entry per record in the file, in file order.**
    ///
    /// Not de-duplicated and not sorted. `crate::package::port::TrustSet::trust`
    /// drops a key it already holds, which is right for a *verifier* and wrong
    /// for a *report*: a store carrying one key twice is a fact about that file,
    /// and a reader that silently folded the two would answer *one key* about a
    /// file holding two records. `AGENTS.md` §5 — report what the system said.
    ///
    /// **Empty is a real answer**, and it is what `ADR-028` §5.5.5's
    /// `--no-trust` image produces: a machine that trusts nobody and can install
    /// nothing. It is not the same as an unreadable store, which refuses.
    pub keys: Vec<TrustedKey>,
}

/// One key a machine trusts, and the limits of that trust. `ADR-028` §5.5.2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedKey {
    /// `BLAKE3` of [`Self::key`] — the value a payload header carries in its
    /// `key_id` field, and therefore **the field that joins an artifact to a
    /// machine.** `ADR-029` §3.
    ///
    /// **Computed by the adapter rather than stored in the file.**
    /// `crate::package::trust_store` says why the file does not carry it: *"it
    /// is BLAKE3 of the public key, so storing it would create a second place
    /// for it to be written down and a way for the two to disagree."*
    pub id: crate::package::port::KeyId,
    /// The key itself, 32 raw bytes. Reported as well as the id, because the
    /// two answer different questions: the id is what a header names, and the
    /// key is what an operator has in a `.pub` file beside their seed.
    pub key: crate::package::port::PublicKey,
    /// The label the store carries. **A name for refusals, never an authority**
    /// — `crate::package::port::TrustEntry::label` carries the rule, and
    /// `ADR-028` §5.5.1 carries the argument for why a label alone would be
    /// worthless.
    pub label: String,
    /// The `<kind>` tokens this key may sign, in the order the file lists them.
    ///
    /// **A key entitled to nothing is a real state**, not a missing value:
    /// `ADR-028` §5.5.4 makes it what a revoked entry becomes before the image
    /// carrying its removal is built.
    pub may_sign: Vec<String>,
}

/// One processor's time, in **milliseconds since boot**.
///
/// The kernel states these in `USER_HZ` ticks; the conversion happens once, in
/// the adapter, for [`MemoryRead`]'s reason — nothing downstream should have to
/// carry a unit that is a property of a C header.
///
/// **Every field is cumulative and none of them is a percentage.** See
/// [`CpuRead`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CpuTimes {
    /// `cpu`, `cpu0`, `cpu1`… as the kernel names the line.
    pub name: String,
    pub user_millis: u64,
    /// Time in user mode at a lowered priority. Counted **separately from**
    /// `user_millis` by the kernel and kept separate here, because a machine
    /// whose work is all niced is a machine that is yielding on demand.
    pub nice_millis: u64,
    pub system_millis: u64,
    /// **The one that makes a machine look busy when it is not**: idle time is
    /// what a utilisation is one-minus.
    pub idle_millis: u64,
    /// Waiting on I/O. On a Pi this is the number to look at beside a `D`-state
    /// process — `ADR-018` §7's failing card shows up in both.
    ///
    /// **The kernel's own documentation calls this figure unreliable**, and it
    /// is reported rather than hidden for [`MemoryRead::free_bytes`]'s reason:
    /// it is a fact the kernel states.
    pub iowait_millis: u64,
    pub irq_millis: u64,
    pub softirq_millis: u64,
    /// Time this machine was ready to run and a hypervisor ran something else.
    /// **Zero on bare metal**, which is what a Rivet is, and non-zero would
    /// itself be the finding.
    pub steal_millis: u64,
}

impl CpuTimes {
    /// Every millisecond this processor has accounted for since boot.
    ///
    /// **The denominator of any utilisation computed from two readings**, and
    /// it is summed here rather than by each caller for `get process`'s
    /// `userland` reason: a computation nobody makes the same way twice is one
    /// that disagrees with itself.
    #[must_use]
    pub const fn total_millis(&self) -> u64 {
        self.user_millis
            .saturating_add(self.nice_millis)
            .saturating_add(self.system_millis)
            .saturating_add(self.idle_millis)
            .saturating_add(self.iowait_millis)
            .saturating_add(self.irq_millis)
            .saturating_add(self.softirq_millis)
            .saturating_add(self.steal_millis)
    }

    /// The milliseconds that were **not** idle — busy plus iowait excluded.
    ///
    /// `idle` and `iowait` are both *not running anything*, and a utilisation
    /// that counted iowait as busy would report a machine blocked on a dying SD
    /// card as fully loaded. They are separated here so a caller cannot make
    /// that mistake by accident: this is work, and `iowait_millis` is waiting.
    #[must_use]
    pub const fn busy_millis(&self) -> u64 {
        self.total_millis()
            .saturating_sub(self.idle_millis)
            .saturating_sub(self.iowait_millis)
    }
}

/// The kernel's load average, in **hundredths** — `0.52` arrives as `52`.
///
/// [`MemoryPressure`]'s unit and its reason: `crate::value::Value` has no
/// floating-point variant, and whole numbers would collapse every load below
/// one into zero on a machine where that is the entire interesting range.
///
/// # It is not a CPU utilisation, on Linux specifically
///
/// The number counts tasks that are **runnable or in uninterruptible sleep**.
/// Every other Unix counts only runnable ones. So on a Pi with a failing SD
/// card, the load average climbs to 4 while the processors sit idle — and a
/// reader who learned the number on any other system will read that as a busy
/// machine. `ProcessInfo::state`'s `D` is the same fact seen one process at a
/// time, and the two together are what identify it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadAverage {
    pub avg1: u32,
    pub avg5: u32,
    pub avg15: u32,
    /// Tasks runnable right now, as the kernel counted them at the instant the
    /// file was read.
    pub runnable: u32,
    /// Tasks that exist. On a Rivet this should be a short list — see
    /// `crate::operation::process`.
    pub existing: u32,
}

/// What a machine says about its own heat and clock — `ADR-035` phase 4, and
/// the survey's §7.
///
/// **Three readings that fail independently, and each says so on its own.** A
/// board with a thermal zone and no `cpufreq`, or with both and no route to the
/// firmware, is an ordinary board rather than a fault — so this type has room
/// for each half to be missing *and to say why*, and no room at all for a
/// missing half to arrive as a zero.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThermalRead {
    /// Every thermal zone the machine exposes, in the order the implementor
    /// listed them. Empty is a true answer on hardware that has none.
    pub zones: Vec<ThermalZone>,
    /// Zones that were listed and could not be read, and what the system said
    /// about each. **Never silently dropped**, for
    /// [`StorageRead::unreadable`]'s reason: an answer that omits a zone says
    /// *there is no such zone*.
    pub zones_unreadable: Vec<UnreadableZone>,
    /// Where the zones were looked for, whether or not any were found — so an
    /// empty list names a place rather than shrugging.
    pub zone_source: String,
    /// This machine's clock against its ceiling, or `None` where there is no
    /// `cpufreq` at all.
    pub frequency: Option<CpuFrequency>,
    /// Why [`Self::frequency`] is `None`, in the system's own words. Present
    /// exactly when that is.
    pub frequency_unmeasured: Option<String>,
    /// The firmware's throttle flags, where a route to them exists.
    pub throttle: Option<Throttle>,
    /// **Why the flags were not read** — and the reason this field exists
    /// rather than a `Throttle::default()`.
    ///
    /// A `Throttle` of all-`false` asserts *this board has never been
    /// under-volted or throttled since it booted*. That is the most valuable
    /// single claim in this whole subject (survey §7.1), and a build that could
    /// not reach the firmware has not established it. Present exactly when
    /// [`Self::throttle`] is `None`.
    pub throttle_unmeasured: Option<String>,
}

/// One thermal zone, as the machine named it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThermalZone {
    /// The zone's own name for itself — `cpu_thermal` on a Pi 4, `coretemp` on
    /// x86. Copied verbatim (`AGENTS.md` §5) rather than mapped onto a
    /// vocabulary this build made up.
    pub name: String,
    /// **Thousandths of a degree Celsius**, which is the unit Linux states and
    /// therefore the unit that travels. [`crate::value::Value`] has no
    /// floating-point variant, so a conversion to degrees here would either
    /// lose the fraction or invent a representation for it.
    pub millidegrees: i64,
}

/// One thermal zone that was listed and could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadableZone {
    /// Which one, so a zone missing from the answer is distinguishable from a
    /// zone this machine does not have.
    pub zone: String,
    /// What the system said, not what it was expected to say.
    pub why: String,
}

/// What the processor is clocked at, and what it could be clocked at.
///
/// **Hertz, not the kilohertz `cpufreq` states.** The conversion happens once,
/// in the adapter, for [`MemoryRead`]'s reason: a unit forwarded is a unit every
/// reader has to remember.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuFrequency {
    /// What it is running at now — `scaling_cur_freq`.
    pub current_hz: u64,
    /// What the **hardware** can do — `cpuinfo_max_freq`. Fixed for a given
    /// board and governor-independent.
    pub maximum_hz: u64,
    /// What the hardware's floor is — `cpuinfo_min_freq`.
    pub minimum_hz: Option<u64>,
    /// **The ceiling policy currently allows** — `scaling_max_freq`, which is
    /// not the same fact as [`Self::maximum_hz`] and is the one that moves.
    pub ceiling_hz: Option<u64>,
    /// The governor's name, verbatim. It is what decides whether a low
    /// [`Self::current_hz`] is a throttle or an idle machine being sensible.
    pub governor: Option<String>,
    /// Which `cpufreq` directory this was read from. One CPU's, and the answer
    /// names it rather than implying the whole package.
    pub source: String,
}

/// The firmware's own account of whether it has held this machine back.
///
/// **Eight booleans in two halves, and the second half is why this is worth
/// building.** The `_now` four are a spot reading that anything could take; the
/// `_since_boot` four are sticky, and they are the only record that a board
/// which is fine at 09:00 spent 03:00 under-volted (survey §7.1).
///
/// **`false` here means measured-and-false.** A build that could not reach the
/// firmware reports [`ThermalRead::throttle`] as `None` and says why in
/// [`ThermalRead::throttle_unmeasured`]; it never fills this in with zeroes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Throttle {
    /// The supply is below the threshold **right now**. Survey §7.1 calls
    /// under-voltage *"the Pi's actual killer"* — a marginal PSU or a cheap
    /// cable, and a leading cause of card corruption.
    pub under_voltage_now: bool,
    /// The clock is being held below its maximum right now.
    pub frequency_capped_now: bool,
    /// The firmware is throttling right now.
    pub throttled_now: bool,
    /// The soft temperature limit is active right now.
    pub soft_temperature_limit_now: bool,
    /// **Under-voltage has occurred since this machine booted.** The single
    /// most valuable fact this subject carries, because nothing else recovers
    /// it once the condition has passed.
    pub under_voltage_since_boot: bool,
    /// The clock has been capped since boot.
    pub frequency_capped_since_boot: bool,
    /// The firmware has throttled since boot.
    pub throttled_since_boot: bool,
    /// The soft temperature limit has been hit since boot.
    pub soft_temperature_limit_since_boot: bool,
    /// **The word the firmware actually returned**, kept whole.
    ///
    /// `AGENTS.md` §5: report what the system said, not what it was expected to
    /// say. A bit this build does not know about is invisible in the eight
    /// booleans and visible here, which is the difference between a surface
    /// that can be checked against the vendor's documentation and one that
    /// cannot.
    pub raw: u32,
    /// Which interface answered — a `/dev` node, a sysfs path, whatever it was.
    pub source: String,
}

/// What the kernel says about the boot that is running — `ADR-035` phase 4.
///
/// **No wall-clock time appears here, and that is the design.** `ADR-021` gives
/// this hardware no real-time clock, so a timestamp is a claim and a duration
/// from the monotonic counter is a fact. See [`Observed::read_boot`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BootRead {
    /// This boot's identifier, as the kernel minted it.
    ///
    /// **A random UUID, which is why it does not sort.** It is the same value
    /// [`crate::journal::Record::boot`] carries, so the two can be compared —
    /// and a machine whose journal does not mention its own current boot is
    /// worth noticing.
    pub boot_id: Option<String>,
    /// Why [`Self::boot_id`] is `None`. Present exactly when it is.
    pub boot_id_unmeasured: Option<String>,
    /// Whole seconds since this boot began.
    ///
    /// **Not optional**, because it is the one fact this subject cannot be
    /// useful without: an implementor that cannot read it refuses instead.
    pub uptime_seconds: u64,
    /// Whole seconds this machine has spent idle, summed across cores — so on a
    /// four-core board an idle minute counts four. `None` where the kernel
    /// states no such figure.
    pub idle_seconds: Option<u64>,
    /// Where the readings came from, so that the answer names a place.
    pub source: String,
}

/// Which A/B slot is running, from **two** independent authorities —
/// `ADR-035` phase 4. See [`Observed::read_slots`].
///
/// **Every one of the three readings can fail on its own**, and each says so
/// separately: a development host has no device tree, a machine booted some
/// other way has no `PARTLABEL=` on its command line, and a platform with no
/// one-shot boot has no trial to be in. None of those is a fault and none of
/// them may be reported as a slot.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SlotRead {
    /// Which **boot** partition the firmware read, as the firmware states it.
    pub booted: Option<crate::state::Slot>,
    /// Why [`Self::booted`] is `None`, in the system's own words.
    pub booted_unmeasured: Option<String>,
    /// Where the firmware's statement was read from.
    pub booted_from: String,
    /// **The `root=` argument the kernel was given, verbatim.**
    ///
    /// A string rather than a [`crate::state::Slot`], and that is the seam:
    /// what an adapter knows is *which file, in what format*; what the domain
    /// knows is that `PARTLABEL=system-a` names slot A
    /// (`crate::state::Slot::system_partlabel`). An adapter that resolved it
    /// would be a second place the label is spelled.
    ///
    /// Copied unchanged (`AGENTS.md` §5), so a machine booted by `root=/dev/…`
    /// or `root=UUID=…` reports what it was actually told rather than nothing.
    pub root_argument: Option<String>,
    /// Why [`Self::root_argument`] is `None`.
    pub root_unmeasured: Option<String>,
    /// Where the command line was read from.
    pub root_from: String,
    /// Whether this boot is a **one-shot trial** — `ADR-018` §5's `tryboot`.
    ///
    /// `None` where the platform has no such concept or it could not be read;
    /// **never `false` for an unread flag**, because `false` says *this is a
    /// committed boot* and a trial that is not committed rolls back.
    pub in_trial_boot: Option<bool>,
    /// Why [`Self::in_trial_boot`] is `None`.
    pub trial_unmeasured: Option<String>,
    /// **Which slot an ordinary restart would boot** — the default, read from
    /// the platform's own record rather than inferred.
    ///
    /// Added 2026-09-05, and it exists because [`Self::in_trial_boot`] answers
    /// a question nobody was asking. That flag says **how this boot was
    /// entered**; it does not and cannot say whether the boot has since been
    /// *committed*, because committing is `cmd/rivet-initd`'s `confirm` step
    /// rewriting the default-slot record while the machine runs. The device
    /// tree's `tryboot` property was set by the firmware before any of that and
    /// never changes afterwards.
    ///
    /// So a healthy trial that PID 1 has already confirmed reports
    /// `in_trial_boot: true` **for the rest of its life**, beside prose warning
    /// that an ordinary restart will roll it back — which is false, and is the
    /// opposite of what will happen.
    ///
    /// **Three readers have been misled by it**, two of them agents raising the
    /// same false alarm on separate days and the third this session, during a
    /// live recovery where the answer decided whether the maintainer's display
    /// came back. `AGENTS.md` §7 both ways: a guard that reports a stale
    /// reading is worse than no guard, because it is believed.
    ///
    /// With this, `committed` is a comparison the domain can make — see
    /// [`SlotRead::committed`] — and the prose can say which of the two
    /// situations a reader is actually in.
    pub default_slot: Option<crate::state::Slot>,
    /// Why [`Self::default_slot`] is `None`.
    pub default_unmeasured: Option<String>,
}

impl SlotRead {
    /// **Whether a restart comes back here**, which is the question a reader
    /// asking about a trial boot actually has.
    ///
    /// `Some(true)` when the default slot is the running one — the boot is
    /// committed, however it was entered. `Some(false)` when they differ, which
    /// is an uncommitted trial and the case where doing nothing rolls back.
    /// `None` when either side was not measured, because guessing here is how
    /// the confusion this method exists to end got started.
    #[must_use]
    pub fn committed(&self) -> Option<bool> {
        match (self.default_slot, self.booted) {
            (Some(default), Some(running)) => Some(default == running),
            _ => None,
        }
    }
}

/// What a machine says about its own networking — `ADR-035` phase 4.
///
/// See [`Observed::read_network`]. Nothing here names a socket, a netlink
/// message or an address family constant: an address is a string the machine
/// printed and a counter is a number it gave.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NetworkRead {
    /// Every interface, **in name order**, so two readings of an unchanged
    /// machine produce the same bytes.
    pub interfaces: Vec<Interface>,
    /// Interfaces that were listed and could not be read, with the reason.
    /// Never dropped — [`StorageRead::unreadable`]'s rule.
    pub interfaces_unreadable: Vec<UnreadableInterface>,
    /// Where the interfaces were looked for.
    pub interface_source: String,
    /// The routing table, **in the order the machine listed it**.
    pub routes: Vec<Route>,
    /// Where the routes came from.
    pub route_source: String,
    /// The resolvers this machine would ask, **in order**, verbatim.
    pub nameservers: Vec<String>,
    /// Where the resolvers came from.
    pub nameserver_source: String,
    /// **Everything this reading did not measure, and why.**
    ///
    /// The field that keeps the rest honest. A network answer with no IPv4
    /// address in it reads as *this machine has no IPv4 address* — on a board
    /// you reached over IPv4. Whatever an implementor could not get, it names
    /// here rather than leaving a shaped hole.
    pub unmeasured: Vec<Unmeasured>,
}

/// One network interface, as the machine describes itself.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Interface {
    pub name: String,
    /// The kernel's own word — `up`, `down`, `unknown`, `dormant`,
    /// `lowerlayerdown`. **Verbatim** (`AGENTS.md` §5) rather than mapped onto
    /// a boolean: `unknown` is what a `tun` device reports and is not `down`,
    /// and collapsing the two loses the distinction that matters.
    pub operational_state: String,
    /// Whether the kernel sees a carrier — a cable, or an association.
    ///
    /// **`None` is not `false`.** The kernel refuses this file outright on an
    /// administratively-down interface, and `false` would say *the cable is
    /// out* about an interface nobody brought up.
    pub carrier: Option<bool>,
    /// The link-layer address as the machine printed it.
    pub hardware_address: Option<String>,
    pub mtu: Option<u32>,
    /// Negotiated link speed in megabits per second.
    ///
    /// **`None` where the machine does not know**, which includes every
    /// interface that is down and every virtual one. The kernel spells that
    /// `-1`; carrying the `-1` outward would make it a speed.
    pub speed_megabits: Option<u64>,
    /// Addresses on this interface, in the order the machine listed them.
    pub addresses: Vec<InterfaceAddress>,
    /// Bytes, packets, errors and drops. `None` where the machine keeps none.
    pub counters: Option<InterfaceCounters>,
}

/// One address on an interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceAddress {
    /// The address, as text, exactly as the machine rendered it.
    pub address: String,
    /// How many leading bits are the network part.
    pub prefix_length: u32,
    /// `inet` or `inet6`. A string rather than an enum because it is a word the
    /// machine used and this crate names no address families.
    pub family: String,
    /// `global`, `link`, `host` — verbatim where the machine says, `None` where
    /// it does not.
    pub scope: Option<String>,
}

/// What an interface has carried since it came up.
///
/// **The errors and drops are the half worth having**, and the survey's §7.4
/// says why: *"per-interface counters — errors, drops, link speed"* answer
/// *is the cable bad* directly, and nothing else on this machine does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InterfaceCounters {
    pub received_bytes: u64,
    pub transmitted_bytes: u64,
    pub received_packets: u64,
    pub transmitted_packets: u64,
    pub received_errors: u64,
    pub transmitted_errors: u64,
    pub received_dropped: u64,
    pub transmitted_dropped: u64,
}

/// One routing-table entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    /// The destination network as text. `0.0.0.0` with a zero prefix is the
    /// default route, and it is left as the machine stated it rather than
    /// renamed to `default` — the domain can say *default* where a human reads
    /// the answer.
    pub destination: String,
    pub prefix_length: u32,
    /// The next hop, or `None` for a directly-connected network.
    pub gateway: Option<String>,
    /// Which interface it leaves by.
    pub interface: String,
    pub metric: u32,
    /// `inet` or `inet6`.
    pub family: String,
}

/// One interface that was listed and could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadableInterface {
    pub name: String,
    /// What the system said, not what it was expected to say.
    pub why: String,
}

/// One fact a reading did **not** establish, and why.
///
/// **Not an error and not a warning.** It is the difference between *"this
/// machine has no such thing"* and *"nobody looked"*, which is the distinction
/// `AGENTS.md` §5 is about and the one an answer silently loses by omission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unmeasured {
    /// What was not measured, in the answer's own vocabulary.
    pub what: String,
    /// Why not, and — where there is one — what would measure it.
    pub why: String,
}

/// Every block device this machine has — `ADR-035` phase 4. See
/// [`Observed::read_devices`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeviceRead {
    /// Every block device, **in name order**, so two readings of an unchanged
    /// machine produce the same bytes.
    pub devices: Vec<BlockDevice>,
    /// Devices that were listed and could not be read, and why. Never dropped —
    /// [`StorageRead::unreadable`]'s rule.
    pub devices_unreadable: Vec<Unmeasured>,
    /// Where the devices were looked for.
    pub source: String,
}

/// One block device, as the kernel describes it.
///
/// **Almost everything is optional, and that is the shape of `sysfs` rather
/// than caution.** A `loop` device has no model, a whole disk has no partition
/// label, an SD card has no endurance figures, and a device that is going away
/// while it is read has whatever it has. Each absence is an absence rather than
/// a zero.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BlockDevice {
    /// The kernel's name for it — `mmcblk0`, `mmcblk0p3`, `sda1`.
    pub name: String,
    /// `disk` or `partition`, as the device's own `uevent` says. Verbatim, so a
    /// kind this build does not know about is still visible.
    pub kind: Option<String>,
    /// The device node's name, as `uevent` gives it.
    pub node: Option<String>,
    /// **The GPT partition label** — `system-a`, `state`. This is what
    /// `root=PARTLABEL=` names and what a running machine finds `/state` by
    /// (`crate::state::STATE_PARTLABEL`), which makes it the most useful single
    /// string on a partition.
    pub partition_label: Option<String>,
    /// The 1-based partition number, for a partition.
    pub partition_number: Option<u32>,
    /// The whole disk a partition belongs to.
    pub parent: Option<String>,
    /// Size in **bytes**.
    ///
    /// `sysfs` states this in 512-byte units **always** — not in the device's
    /// logical block size, which is the trap. The conversion happens once, in
    /// the adapter, so nothing downstream carries a unit.
    pub size_bytes: Option<u64>,
    /// Whether the kernel says this device is read-only.
    pub read_only: Option<bool>,
    /// Whether the medium is removable.
    pub removable: Option<bool>,
    /// The device's own name for itself, where it has one.
    pub model: Option<String>,
    /// What has been read from and written to it **since boot**.
    pub io: Option<BlockIo>,
    /// **The eMMC's own estimate of how much of its rated life is gone**,
    /// verbatim, or `None`.
    ///
    /// A JEDEC 5.0+ eMMC reports this and **a consumer SD card generally does
    /// not** (survey §7.3). It is carried as the string the device produced
    /// rather than converted into a percentage, because the mapping is the
    /// vendor's and a number invented here would be exactly the fabricated
    /// endurance figure §7.3 refuses.
    pub life_time: Option<String>,
    /// The device's pre-end-of-life warning, verbatim, or `None`. Same rule.
    pub pre_eol_info: Option<String>,
}

/// What a block device has carried since boot.
///
/// **Bytes written is the one that matters here**, and survey §7.3 says why:
/// `ADR-018` §7 already treats SD wear as a design constraint — the
/// journal-on-disk decisions were all taken for it — so measuring the actual
/// write volume closes the loop on decisions taken blind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BlockIo {
    pub read_bytes: u64,
    pub written_bytes: u64,
    pub reads_completed: u64,
    pub writes_completed: u64,
}

/// What an implementor of [`Observed::read_filesystems`] found.
///
/// **`mount_table` is not decoration**, for `JournalRead::files`' reason: an
/// answer that reported only filesystems could not tell *"this machine has these
/// filesystems"* from *"this machine's mount table is not where I looked"*.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StorageRead {
    /// Every filesystem that was in the table and could be measured, **in the
    /// order the table listed them**.
    ///
    /// Order is the mount order, and it is load-bearing rather than incidental:
    /// two filesystems may share a mount point, and the later one is the one a
    /// path actually reaches. `crate::operation::storage` takes the last
    /// match for exactly that reason, so an implementor that sorted this list
    /// would make it name the shadowed filesystem.
    pub filesystems: Vec<MountedFilesystem>,
    /// Entries of the table whose usage could not be read, and what the system
    /// said about each. **Never silently dropped** — see
    /// [`Observed::read_filesystems`].
    pub unreadable: Vec<UnreadableFilesystem>,
    /// Where the mount table was read from, whether or not anything was in it.
    pub mount_table: String,
}

/// One mounted filesystem, as the machine reported it.
///
/// Every field is either a string the machine produced or a count it gave. The
/// arithmetic — total, used, available, capacity — is
/// `crate::operation::storage`'s, and [`Observed::read_filesystems`] carries
/// the argument for the split.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MountedFilesystem {
    /// What it is, as the mount table's first column says: a device node, or a
    /// word like `tmpfs` for a filesystem with no backing device. Copied
    /// verbatim (`AGENTS.md` §5) rather than normalised.
    pub source: String,
    /// Where it is mounted.
    pub mount_point: String,
    /// The filesystem type the machine reported — `ext4`, `erofs`, `tmpfs`.
    pub fstype: String,
    /// Whether the *kernel* says this filesystem is read-only.
    ///
    /// Read from the filesystem's own flags rather than from the mount option
    /// string, because the two can differ: a card that has gone read-only
    /// mid-write is the failure `ADR-018` §8 is about, and it changes the
    /// kernel's flag without rewriting anything an operator typed.
    pub read_only: bool,
    /// The size in bytes of the block [`Self::blocks`] counts.
    ///
    /// **The fragment size, not the I/O block size**, where a machine
    /// distinguishes them. They are usually equal and are not always, and using
    /// the wrong one scales every number here.
    pub block_bytes: u64,
    /// Total blocks in the filesystem.
    pub blocks: u64,
    /// Blocks not in use — **including the reserve**, which an ordinary caller
    /// cannot have. See [`Self::blocks_available`].
    pub blocks_free: u64,
    /// Blocks an unprivileged caller could still use. This is the number that
    /// answers *"how much room is left"*, and it is smaller than
    /// [`Self::blocks_free`] by the filesystem's reserve.
    pub blocks_available: u64,
    /// Total inodes, or `0` on a filesystem that has no fixed number of them.
    /// Zero means *not applicable*, not *none left*.
    pub inodes: u64,
    /// Inodes not in use.
    pub inodes_free: u64,
}

/// One entry of the mount table that could not be measured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadableFilesystem {
    /// Which entry, so an operator can tell a filesystem that is missing from
    /// the answer from one that was never mounted.
    pub mount_point: String,
    /// What the system said, not what it was expected to say (`AGENTS.md` §5).
    pub why: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both kinds of restart must have a form an agent reads and a sentence an
    /// operator reads, and neither pair may collide: performing the wrong
    /// restart trials nothing and looks like success.
    #[test]
    fn the_two_restarts_are_distinguishable_in_both_readings() {
        assert_ne!(Reboot::OneShot.as_str(), Reboot::Ordinary.as_str());
        assert_ne!(
            Reboot::OneShot.requirement(),
            Reboot::Ordinary.requirement()
        );
        for kind in [Reboot::OneShot, Reboot::Ordinary] {
            assert!(!kind.as_str().is_empty());
            assert!(!kind.requirement().is_empty());
            assert_eq!(kind.to_string(), kind.as_str());
        }
    }

    /// The one-shot's sentence has to say the thing that is easy to get wrong,
    /// because an ordinary restart on this hardware boots the default slot.
    #[test]
    fn the_one_shot_requirement_says_that_an_ordinary_restart_will_not_do() {
        assert!(Reboot::OneShot.requirement().contains("tryboot"));
        assert!(Reboot::OneShot.requirement().contains("ordinary restart"));
    }

    #[test]
    fn both_kinds_of_machine_error_carry_their_message_out_unchanged() {
        for error in [
            MachineError::Unsafe("interlock".to_string()),
            MachineError::Refused("EROFS".to_string()),
        ] {
            assert_eq!(error.to_string(), error.why());
            assert!(!error.why().is_empty());
        }
        assert_ne!(
            MachineError::Unsafe("x".to_string()),
            MachineError::Refused("x".to_string()),
            "the two kinds must not compare equal; a caller branches on them"
        );
    }

    // -- stopping the machine --------------------------------------------------

    /// **The contract, written out.** `ADR-017` §3.1.10's table has three rows
    /// and this enum is the request half of it; a fourth way to stop a Rivet
    /// machine would be a fourth signal pid 1 does not handle.
    #[test]
    fn the_three_ways_to_stop_a_machine_are_adr_017_3_1_10s_three_rows() {
        let words: Vec<&str> = Stop::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(words, ["power-off", "restart", "trial-restart"]);
    }

    /// Every one has a word and a sentence, and no two of either collide: an
    /// operator who confirms the wrong one gets a machine in a state they did
    /// not ask for, and one of the three does not come back at all.
    #[test]
    fn no_two_ways_to_stop_read_alike_in_either_reading() {
        for (i, a) in Stop::ALL.iter().enumerate() {
            assert!(!a.as_str().is_empty());
            assert!(!a.means().is_empty());
            assert_eq!(a.to_string(), a.as_str());
            assert!(!a.as_str().starts_with('-'), "{a} looks like a flag");
            for b in Stop::ALL.iter().skip(i + 1) {
                assert_ne!(a.as_str(), b.as_str());
                assert_ne!(a.means(), b.means(), "{a} and {b} say the same thing");
            }
        }
    }

    /// **Exactly one of the three carries the firmware's one-shot flag.** If
    /// `Restart` ever claimed it, an operator would arm a trial boot, restart,
    /// come back on the default slot, and read a healthy board as a successful
    /// trial — the silent success `ADR-018` §5 exists to make impossible.
    #[test]
    fn only_the_trial_restart_carries_the_one_shot_flag() {
        let carrying: Vec<&Stop> = Stop::ALL
            .iter()
            .filter(|s| s.carries_the_one_shot())
            .collect();
        assert_eq!(carrying, vec![&Stop::TrialRestart]);
    }

    /// The sentence for the one-shot has to say the thing that is easy to miss:
    /// that doing nothing is what rolls it back.
    #[test]
    fn the_trial_restarts_sentence_says_what_happens_when_nothing_is_committed() {
        let means = Stop::TrialRestart.means();
        assert!(means.contains("ONCE"), "{means}");
        assert!(means.contains("not committed"), "{means}");
        assert!(means.contains("ADR-018 §5.1"), "{means}");
        // And the ordinary restart must say the opposite, because an operator
        // who confirmed it after arming has trialled nothing.
        assert!(
            Stop::Restart.means().contains("NOT taken"),
            "{}",
            Stop::Restart.means()
        );
    }
}
