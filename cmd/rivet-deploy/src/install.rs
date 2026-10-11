//! `ADR-029` §9.2's install sequence, **performed**.
//!
//! The decisions are elsewhere and none of them is here:
//!
//! | Step | Whose |
//! | :-- | :-- |
//! | 1 — read and verify the header | `rivet_core::package::verify::header`, over `read::open` |
//! | 2 — refuse on product / arch / platform / kind | `rivet_core::package::update::Payload::accepts` |
//! | 3 — **report** a non-newer version, never refuse | here, and it reports the two strings without ordering them — `ADR-029` §12 item 7 |
//! | 4 — running slot → other slot → its payload target | `rivet_adapter_image::target_slot`, over `BootSlots` |
//! | 4a — refuse unless this boot is committed, because otherwise the target IS the slot a reset boots | [`uncommitted_trial`], over `BootSlots::default_slot` — `ADR-018` §5 |
//! | 5 — fit, and reserved names | `InstallPlan::for_slot` and `Payload::open` |
//! | 6 — verify **every** member before any write | `rivet_core::package::verify::members`, over this binary's `Kernel` |
//! | 7 — write the system member, `fsync` | here, over `Kernel::write_device` |
//! | 8 — write the boot half, skipping identical files | here, over `InstallPlan` and `action_for` |
//! | 9 — read the written partition back and hash it | here |
//! | 10 — arm | `BootSlots::arm_trial_boot` |
//! | 11 — the new slot boots and PID 1 confirms | `cmd/rivet-initd`, unchanged |
//!
//! What this module owns is **the order**, and the order is the format's whole
//! security argument. Two properties it must have, and both are structural
//! rather than commented:
//!
//! 1. **Nothing is written before every member has verified.** Step 7 takes a
//!    `VerifiedMember`, which a caller cannot construct — so an install that
//!    wrote first would not compile.
//! 2. **The system member is written before the boot half.** `ADR-029` §9's
//!    second bound on F5: the dangerous write is the shortest-lived. The plan
//!    holds the two in separate fields, so there is no list to iterate the wrong
//!    way round.
//!
//! # What a failure leaves behind
//!
//! `ADR-029` F3: *"the slot is partially written and **inert**. No selector
//! moved. Report and leave it. **Do not wipe it** — wiping is a second
//! destructive operation on the one thing whose virtue is being unreachable."*
//! **Nothing in this module deletes or zeroes anything**, and that is a
//! deliberate absence rather than an omission.

use rivet_adapter_image::{
    ArmRefusal, ArmedReboot, BootFileAction, BootHalf, BootSlots, BootWrite, InstallPlan,
    AUTOBOOT_FILE, AUTOBOOT_LOCK, Mounted, PAYLOAD_BOOT_MOUNT, PayloadTarget, SlotId, action_for,
    autoboot_partition_device, slot_named, target_slot,
};
use rivet_core::journal::{Severity, Value};
use rivet_core::package::error::PackageError;
use rivet_core::package::install::{FILE_MODE, Step};
use rivet_core::package::port::{Digest, TrustSet, Verify};
use rivet_core::package::read::{Package, VerifiedMember};
use rivet_core::package::trust_store;
use rivet_core::package::update::Payload;
use rivet_core::package::verify::{self, Fault, Reading, TrustRefusal};

use crate::journal::{Journal, count, field};
use crate::kernel::{Errno, Kernel};
use crate::medium::KernelMedium;
use rivet_core::package::Kind;
use rivet_core::package::port::MaySign;

/// What this machine claims to be, for `ADR-029` §9.2 step 2.
///
/// Every field is supplied by the composition root rather than written here, so
/// that a test can drive the mismatches.
///
/// **There is no `arch`, and there was until 2026-09-12.** It was
/// `std::env::consts::ARCH` — the architecture this binary was compiled for —
/// which is `arm` on the `armv6` board `ADR-027` §3 names, so it would have
/// refused every artifact there. The token is the image's now,
/// `rivet_core::package::arch::PATH`, read at the point of use in [`run`] like
/// the trust store beside it (`PLAN-Feature-Set-2026-09-12` §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Machine {
    pub product: String,
    pub platform: String,
    /// The version of the image that is running. **Reported and never
    /// compared** — see [`Outcome`] and `ADR-029` §12 item 7.
    pub version: String,
}

/// How an install ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The slot is written and the next boot will trial it.
    Installed {
        slot: SlotId,
        armed: ArmedReboot,
        system_bytes: u64,
        boot_written: usize,
        boot_skipped: usize,
    },
    /// `--dry-run`: everything verified, nothing written.
    Planned { slot: SlotId, system_bytes: u64 },
    /// An application reached `/state/app` and `current` points at it.
    ///
    /// **No slot and no arming.** An application install changes nothing about
    /// the next boot: `ADR-015` §7's symlink swap is live the moment it happens,
    /// and the supervisor picks the application up on its next convergence.
    InstalledApplication {
        name: String,
        version: String,
        files: usize,
    },
    /// `--dry-run` for an application.
    PlannedApplication {
        name: String,
        version: String,
        steps: usize,
    },
    /// **Shared Rivets, 2026-08-31.** A library reached its content-addressed
    /// directory under `/state/lib`.
    ///
    /// **No `current` and no activation**, which is the shape of the whole
    /// design: the directory name carries a digest of the bytes in it, so there
    /// is no version to select and nothing to swap.
    InstalledLibrary {
        reference: String,
        files: usize,
    },
    /// The library's directory was already there, so nothing was written.
    ///
    /// A **success**: see `rivet_core::deploy::Outcome::LibraryAlreadyInstalled`
    /// for why this is not the refusal its application counterpart is.
    LibraryAlreadyInstalled { reference: String },
    /// `--dry-run` for a library.
    PlannedLibrary { reference: String, steps: usize },
    /// The payload is not one this machine will install. **A different payload
    /// would help**, and nothing was written.
    RefusedPayload(String),
    /// The machine, the card or the kernel refused. A different payload would
    /// not help.
    RefusedMachine(String),
}

impl Outcome {
    /// Did anything reach the card?
    ///
    /// # `None` is an answer, and it used to be a wrong `true`
    ///
    /// **Corrected 2026-08-24.** This returned a plain `bool` and put
    /// [`Self::RefusedMachine`] in the `true` arm, so `main` emitted
    /// `wrote: false` on **every** machine refusal — including one that arrived
    /// part-way through a 640 MiB write, which is the only case in this binary
    /// where an inactive slot is left neither old nor new.
    ///
    /// That is `AGENTS.md` §5 exactly — *report what the system said, not what
    /// it was expected to say* — and the direction is the dangerous one: the
    /// record an operator reads after a failed install asserted that the slot
    /// was untouched. The other three arms are promises `ADR-029` §5's ordering
    /// makes structural (nothing is written until every member verifies), and
    /// they are unchanged.
    ///
    /// `None` reaches the journal as an **omitted** field rather than as a
    /// `false`: `crate::journal`'s `Value` has no null, and `ADR-006` §8.6.3
    /// already ruled that absence is how a value says it has none.
    #[must_use]
    pub fn wrote_nothing(&self) -> Option<bool> {
        match self {
            Self::Planned { .. }
            | Self::PlannedApplication { .. }
            | Self::PlannedLibrary { .. }
            | Self::LibraryAlreadyInstalled { .. }
            | Self::RefusedPayload(_) => Some(true),
            Self::Installed { .. }
            | Self::InstalledApplication { .. }
            | Self::InstalledLibrary { .. } => Some(false),
            Self::RefusedMachine(_) => None,
        }
    }

    /// How this ended, in the vocabulary `rivet get install` reads.
    ///
    /// The six arms of this enum carry *what happened*; `rivet_core::deploy::Outcome`
    /// carries *which of six ways it ended*, as a stable token a record can hold
    /// and a reader in another crate can branch on. One is the installer's
    /// working type and the other is the contract — see that module for why
    /// there are two.
    #[must_use]
    pub const fn ended(&self) -> rivet_core::deploy::Outcome {
        use rivet_core::deploy::Outcome as Ended;
        match self {
            Self::Installed { .. } => Ended::Installed,
            Self::InstalledApplication { .. } => Ended::InstalledApplication,
            Self::Planned { .. } => Ended::Planned,
            Self::PlannedApplication { .. } => Ended::PlannedApplication,
            Self::InstalledLibrary { .. } => Ended::InstalledLibrary,
            Self::LibraryAlreadyInstalled { .. } => Ended::LibraryAlreadyInstalled,
            Self::PlannedLibrary { .. } => Ended::PlannedLibrary,
            Self::RefusedPayload(_) => Ended::RefusedPayload,
            Self::RefusedMachine(_) => Ended::RefusedMachine,
        }
    }
}

/// Everything an install needs that is not the machine.
pub struct Request<'a> {
    pub payload: &'a str,
    pub trusted_keys: &'a [String],
    pub dry_run: bool,
    pub read_back: bool,
}

/// Run one install to its end, saying what happened as it goes.
///
/// `digest` and `verify` are the composition root's — `rivet-crypto`'s BLAKE3
/// and Ed25519 on a machine, fakes in a test. That is `ADR-029` §10's placement:
/// *"this also makes the property that matters — nothing is written before
/// verification — testable against a fake, which a hard crypto dependency would
/// not."*
#[allow(clippy::too_many_arguments)]
pub fn run(
    k: &mut dyn Kernel,
    slots: &dyn BootSlots,
    journal: &mut Journal,
    corr: &str,
    machine: &Machine,
    request: &Request<'_>,
    digest: &mut dyn Digest,
    verify: &dyn Verify,
) -> Outcome {
    match attempt(k, slots, journal, corr, machine, request, digest, verify) {
        Ok(outcome) => outcome,
        Err(outcome) => outcome,
    }
}

/// The body, with `?` available. Every early return is an [`Outcome`], because
/// there is no failure here that is not one.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn attempt(
    k: &mut dyn Kernel,
    slots: &dyn BootSlots,
    journal: &mut Journal,
    corr: &str,
    machine: &Machine,
    request: &Request<'_>,
    digest: &mut dyn Digest,
    verify: &dyn Verify,
) -> Result<Outcome, Outcome> {
    // --- the trust set ------------------------------------------------------
    //
    // **The machine answers unless an operator overrides it** — `ADR-028` §5.8.
    // Until 2026-08-24 this took keys on the command line and nothing else,
    // because nothing put a store in `/system`. `cmd/rivet-image` does now,
    // and §5.5.6's whole argument was that a machine should be able to say what
    // it trusts — which is worth nothing if the thing that installs updates
    // does not read the answer.
    //
    // **Built by `rivet_core::package::verify::trust_set` since 2026-09-12**,
    // the same function the artifact store's door calls (`ADR-052` §7). The
    // three refusals, and the rule that each entry keeps its own mask rather than
    // a narrowed one, are decided there once; what stays here is this binary's
    // remedy, because only this binary has a `--trusted-key` to offer.
    let mut trust = TrustSet::new();
    if request.trusted_keys.is_empty() {
        trust = verify::trust_set(k.read_bytes(trust_store::PATH), digest).map_err(|why| {
            Outcome::RefusedMachine(match why {
                TrustRefusal::Unreadable(_) => format!(
                    "{why}. Pass --trusted-key to name a key for this install, or write a card \
                     that carries the key you sign with."
                ),
                TrustRefusal::Corrupt(_) | TrustRefusal::Empty => format!("{why}."),
            })
        })?;
        for entry in trust.entries() {
            journal.emit(
                k,
                Severity::Debug,
                corr,
                "a key from this machine's trust store",
                vec![
                    field("key_id", Value::Str(entry.id().to_string())),
                    field("label", Value::Str(entry.label().to_string())),
                ],
            );
        }
    }
    for path in request.trusted_keys {
        let bytes = k
            .read_bytes(path)
            .map_err(|e| Outcome::RefusedMachine(format!("cannot read the key at {path}: {e}")))?;
        let raw = rivet_core::package::key_material::parse(&bytes)
            .map_err(|e| Outcome::RefusedMachine(format!("the key at {path}: {e}")))?;
        // **Trusted to sign an update, and nothing else** — `ADR-028` §5.5.
        //
        // This binary writes a 640 MiB system image onto the inactive slot. A
        // key handed to it is entitled to authorise exactly that, so a key held
        // for signing applications cannot reach this code path even if an
        // operator passes it here by mistake. The refusal is
        // `PackageError::KindNotPermitted`, and it fires *after* the signature
        // verifies — which is the point: the signature being genuine is what
        // makes "and yet it may not sign this" worth saying.
        let id = trust.trust(
            digest,
            rivet_core::package::port::PublicKey(raw),
            "operator-supplied",
            MaySign::these(&[Kind::update()]),
        );
        // At WARN, and deliberately louder than the store's own keys: an
        // override means this install was NOT authorised by what the card says
        // it trusts, and that is the sentence somebody will want in the journal
        // afterwards.
        journal.emit(
            k,
            Severity::Warn,
            corr,
            "a key given on the command line is trusted for this install, INSTEAD of this machine's own trust store",
            vec![field("key_id", Value::Str(id.to_string()))],
        );
    }

    // --- what this machine executes, as its image recorded it ----------------
    //
    // **The image's token, never the compiler's** — `rivet_core::package::arch`
    // argues it. Read here, at the point of use and beside the trust store, for
    // the trust store's reason: both are files in `/system` that an image update
    // replaces. A machine whose record cannot be read installs nothing, because
    // the alternative is guessing what it can execute.
    let arch = rivet_core::package::arch::read(k.read_bytes(rivet_core::package::arch::PATH))
        .map_err(|why| Outcome::RefusedMachine(format!("{why}. NOTHING WAS WRITTEN.")))?;

    // --- ADR-029 §5 steps 1-7 ----------------------------------------------
    let file_len = k
        .size_of(request.payload)
        .map_err(|e| Outcome::RefusedMachine(format!("cannot read {}: {e}", request.payload)))?;
    let opened = verify::header(
        &mut PayloadFile::new(k, request.payload),
        file_len,
        digest,
        verify,
        &trust,
    )
    .map_err(|why| fault(request.payload, why))?;
    let (package, head) = (opened.package(), opened.head());
    let header = package.header();
    journal.emit(
        k,
        Severity::Info,
        corr,
        "the payload's header is signed by a key this machine trusts",
        vec![
            field("product", Value::Str(header.product().to_string())),
            field("version", Value::Str(header.version().to_string())),
            field("arch", Value::Str(header.arch().to_string())),
            field(
                "platform",
                Value::Str(header.platform().unwrap_or("").to_string()),
            ),
            field("kind", Value::Str(header.kind().as_str().to_string())),
            field("key_id", Value::Str(header.key_id().to_string())),
            field("members", Value::Int(package.members().len() as i64)),
            field("bytes", count(file_len)),
        ],
    );

    // --- the architecture, for every kind ------------------------------------
    //
    // **`PLAN-Feature-Set-2026-09-12` §3.** Until 2026-09-12 this was compared
    // for a system update only, inside `Payload::accepts`, and an application or
    // library built for another architecture installed and failed when it ran.
    // `verify::architecture` is the same function the store's door asks, so the
    // two cannot disagree about what this machine executes. Before the kind
    // branch, so no kind can be added below this line without it.
    verify::architecture(package, &arch).map_err(|why| {
        Outcome::RefusedPayload(format!("{why} — this machine executes {arch:?}"))
    })?;

    // **Defence in depth for `cmd/rivet-mcpd`'s door check** —
    // `PLAN-Feature-Set-2026-09-12` §3, `.rulings/` R-156 R10. Every
    // artifact reaching this installer has already been through the door, so
    // this fires only for a store an older door named something into, or a
    // path this build's door does not sit in front of. Before the kind branch,
    // exactly like the architecture check above: an application is the only
    // kind this can ever refuse, and `verify::not_an_image_service` answers
    // `Ok(())` for every other kind on its own.
    verify::not_an_image_service(package).map_err(|why| Outcome::RefusedPayload(why.to_string()))?;

    // **Defence in depth for the same door, for a name this project's own
    // machinery already means something else by** — `.rulings/` R-210 D21.
    verify::not_a_reserved_name(package).map_err(|why| Outcome::RefusedPayload(why.to_string()))?;

    // --- the kind decides what happens next --------------------------------
    //
    // Everything above is identical for every artifact kind: the header, the
    // key, the signature, and whether that key may sign THIS kind. `ADR-029`
    // §11 put all three in one container so there would be one verifier; the
    // same argument keeps them in one installer. Below this line the two
    // diverge completely — an application becomes files under `/state/app`, and
    // an update becomes 640 MiB written onto a slot.
    if *package.header().kind() == rivet_core::package::Kind::application() {
        return application(k, journal, corr, request, package, head, digest);
    }
    // **Shared Rivets, 2026-08-31.** A third kind, arriving exactly as the
    // second did: one more branch below a shared verification, and nothing
    // above this line changed to add it.
    if *package.header().kind() == rivet_core::package::Kind::library() {
        return library(k, journal, corr, request, package, head, digest);
    }

    // **Gantry, `ADR-061` §3.** An `apk` verifies exactly like every other
    // kind — the same header, the same signature, the same trust store — and
    // has no meaning on a Rivet machine: it is an Android APK, held by a
    // fleet server's repository so `gantry` can hand it to a phone. Refused
    // by name here, before the slot-selection logic below assumes it is
    // looking at an update.
    if *package.header().kind() == rivet_core::package::Kind::apk() {
        return Err(Outcome::RefusedPayload(
            "an apk is held by a fleet server for a phone to fetch (ADR-061 §3, §6) and is \
             never installed on a Rivet machine"
                .to_string(),
        ));
    }

    // --- step 4: which slot ------------------------------------------------
    let (slot, slot_payload, running, default) = {
        let mut medium = KernelMedium::new(k);
        let running = slots
            .running_slot(&mut medium)
            .map_err(Outcome::RefusedMachine)?;
        let slot = target_slot(slots, &mut medium).map_err(Outcome::RefusedMachine)?;
        let payload = slots.payload(slot).map_err(Outcome::RefusedMachine)?;
        // What an ORDINARY reset boots — the default-slot record, read-only.
        // `BootSlots::default_slot` says why it is not `in_trial_boot`.
        let default = slots.default_slot(&mut medium);
        let said = medium.warnings();
        for line in said {
            journal.emit(k, Severity::Warn, corr, &line, vec![]);
        }
        (slot, payload, running, default)
    };

    // --- not what this machine is already running — PLAN-Feature-Set-2026-09-12 §3 --------------
    //
    // **Found on `rivet-01`, 2026-09-12**: `0.1.85` re-sent to a board running
    // `0.1.85` from slot b was written into slot a, replacing the `0.1.84`
    // rollback with a second copy of the running build. The running slot is the
    // one the firmware reports — the same answer this install's target is
    // computed from, so the refusal and the write can never be about two
    // different slots — and what it holds is the header this installer retained
    // when it wrote it (`rivet_core::deploy::SLOT_HEADERS`). Never a binary hash:
    // two slots holding one build hash alike.
    if let Some(running_slot) = rivet_core::state::Slot::from_boot_partition(running.0)
        && let Ok(retained) = k.read_bytes(&rivet_core::deploy::slot_header_path(running_slot))
    {
        let retained = [(rivet_core::deploy::Running::System(running_slot), retained)];
        if let Some(what) = rivet_core::deploy::already_running(head, &retained) {
            let refusal = rivet_core::deploy::already_running_refusal(request.payload, what);
            journal.emit(k, Severity::Warn, corr, &refusal, vec![]);
            return Ok(Outcome::RefusedPayload(refusal));
        }
    }

    // --- never while a trial is undecided — ADR-018 §5 --------------------
    //
    // **`PLAN-Feature-Set-2026-09-12` §3.** During an uncommitted trial the
    // slot this machine is running is NOT the default, so the slot a reset falls
    // back to is the other one — which is exactly the slot this update would
    // write. A reset or a power cut part-way through that write boots a
    // half-written slot, and a card is what recovers that. Refused before any
    // payload check, because no payload would make it safe.
    if let Some(refusal) = uncommitted_trial(running, slot, default) {
        journal.emit(k, Severity::Warn, corr, &refusal, vec![]);
        return Err(Outcome::RefusedMachine(refusal));
    }
    let PayloadTarget::WholePartition {
        device: system_device,
    } = &slot_payload.system
    else {
        return Err(Outcome::RefusedMachine(
            "this platform's /system target is not a whole partition".to_string(),
        ));
    };
    let system_device = system_device.clone();

    // --- steps 2, 5: the payload is one for this machine and this slot ------
    let reserved = slots.reserved_names(&slot_payload.kernel);
    let update = Payload::open(package, reserved).map_err(refused)?;
    update
        .accepts(&machine.product, &arch, &machine.platform)
        .map_err(refused)?;

    // --- step 3: report the version, and do not order the two ---------------
    //
    // `ADR-028` §6.1 requires a non-newer payload to be REPORTED and never
    // refused, and `ADR-029` §12 item 7 records that nothing in this project
    // says what `0.2.0` versus `0.0.1-g7b84d4b` means. So both strings are put
    // in the record and no comparison is made: a wrong ordering would report a
    // downgrade as an upgrade, which is `AGENTS.md` §5's "a wrong guess does
    // not fail, it succeeds".
    journal.emit(
        k,
        if package.header().version() == machine.version {
            Severity::Warn
        } else {
            Severity::Info
        },
        corr,
        if package.header().version() == machine.version {
            "the payload's version is the one already running"
        } else {
            "the payload's version differs from the one running; this build does not \
             order version strings, so whether it is newer is not asserted"
        },
        vec![
            field(
                "payload_version",
                Value::Str(package.header().version().to_string()),
            ),
            field("running_version", Value::Str(machine.version.clone())),
        ],
    );

    let partition_bytes = k
        .size_of(&system_device)
        .map_err(|e| Outcome::RefusedMachine(format!("cannot size {system_device}: {e}")))?;
    let slot_name = slot_named(slot).map_err(Outcome::RefusedMachine)?;
    let plan = InstallPlan::for_slot(&update, slot, &slot_payload, partition_bytes)
        .map_err(|why| Outcome::RefusedPayload(why.to_string()))?;

    journal.emit(
        k,
        Severity::Info,
        corr,
        "the inactive slot is the target",
        vec![
            field("running_partition", Value::Int(i64::from(running.0))),
            field("target_partition", Value::Int(i64::from(slot.0))),
            field(
                "target_slot",
                Value::Str(format!("{slot_name:?}").to_lowercase()),
            ),
            field("system_device", Value::Str(system_device.clone())),
            field("partition_bytes", count(partition_bytes)),
            field("boot_device", Value::Str(plan.boot().device.clone())),
            field("boot_members", Value::Int(plan.boot().files.len() as i64)),
        ],
    );

    // **ADR-029 §9.3's F5, said out loud at the moment it applies.**
    //
    // **`Info` and not `Warn` since 2026-08-29, and the demotion is the point.**
    // This fires on EVERY update that targets slot A — it is a standing property
    // of the direction, not an exception — and `PLAN.md` §94.2 found what that
    // costs: the board had been emitting it on every install and it had
    // **become wallpaper**, which is how §9.3's mitigation went six days unruled
    // while the machine said so each time.
    //
    // The maintainer's doctrine, ruled the same day (`PLAN.md` §98): WARNING is
    // reserved for something exceptional happening, the operating system's
    // chance to object before the operator damages the machine (.rulings/
    // R-030). A condition that holds on every run of a normal operation is not
    // that.
    // `Warn` here now means the selector **was actually damaged**, which
    // `repair_autoboot` emits and which is genuinely exceptional.
    if plan.targets_the_fallback_partition() {
        journal.emit(
            k,
            Severity::Info,
            corr,
            "this update writes the boot partition the firmware falls back to when \
             autoboot.txt will not parse, so a torn write here boots the half-written \
             slot rather than staying on this one. ADR-029 §9.3's mitigation — detect \
             and repair — is ruled and built (§9.3.1): the selector is read before the \
             boot half is written and restored if it changes, which NARROWS this window \
             rather than closing it",
            vec![field("boot_device", Value::Str(plan.boot().device.clone()))],
        );
    }

    // --- step 6: verify EVERY member, before step 7 writes anything ---------
    let verified: Vec<VerifiedMember<'_>> =
        verify::members(&mut PayloadFile::new(k, request.payload), package, digest)
            .map_err(|why| fault(request.payload, why))?;
    journal.emit(
        k,
        Severity::Info,
        corr,
        "every member's bytes are the ones the signature covers",
        vec![field("members", Value::Int(verified.len() as i64))],
    );

    if request.dry_run {
        // The arming is checked as well, because confirmation happens in a
        // *second* invocation (`ADR-012` §7.1) — a plan that had not looked at
        // the card would show a change, take the confirmation, and only then
        // discover the card cannot carry it.
        let mut medium = KernelMedium::new(k);
        let can = slots.can_arm_trial_boot(&mut medium, slot);
        let said = medium.warnings();
        for line in said {
            journal.emit(k, Severity::Warn, corr, &line, vec![]);
        }
        can.map_err(arm_refused)?;
        journal.emit(
            k,
            Severity::Info,
            corr,
            "a dry run: the payload verifies, the slot would take it, and the card \
             would accept the arming. Nothing was written",
            vec![field("system_bytes", count(plan.system().bytes))],
        );
        return Ok(Outcome::Planned {
            slot,
            system_bytes: plan.system().bytes,
        });
    }

    // --- the slot's record names nothing while the slot is being written ----
    //
    // `rivet_core::deploy::SLOT_HEADERS`. **Emptied before the first byte**, so
    // a write that dies part-way leaves a slot no record claims — never one
    // whose record still names the build being overwritten. A record that
    // cannot be emptied refuses the install here, with the slot untouched:
    // proceeding would leave a record that disagrees with its slot, and the
    // disagreement would later refuse a legitimate install as "already running".
    forget_what_the_slot_holds(k, slot_name)?;

    // --- step 7: the system member, whole ----------------------------------
    let system = plan.system();
    let token = verified
        .iter()
        .copied()
        .find(|v| v.member().index() == system.member_index)
        .ok_or_else(|| {
            Outcome::RefusedMachine("the verified system member went missing".to_string())
        })?;
    write_whole_partition(k, token, request.payload, &system.device)
        .map_err(|e| Outcome::RefusedMachine(format!("cannot write {}: {e}", system.device)))?;
    journal.emit(
        k,
        Severity::Info,
        corr,
        "the system member is written to the inactive slot and synced",
        vec![
            field("device", Value::Str(system.device.clone())),
            field("bytes", count(system.bytes)),
        ],
    );

    // --- step 9: read it back and hash it ----------------------------------
    if request.read_back {
        let found = digest_of_range(k, &system.device, 0, system.bytes, digest).map_err(|e| {
            Outcome::RefusedMachine(format!("cannot read {} back: {e}", system.device))
        })?;
        if found != system.digest {
            return Err(Outcome::RefusedMachine(format!(
                "{} does not hold what the signed header says it should. The slot is \
                 left as it is — ADR-029 F3: a partial slot is inert and wiping it is a \
                 second destructive operation on the one thing whose virtue is being \
                 unreachable",
                system.device
            )));
        }
        journal.emit(
            k,
            Severity::Info,
            corr,
            "the written slot reads back as the digest the signature covers",
            vec![field("device", Value::Str(system.device.clone()))],
        );
    } else {
        journal.emit(
            k,
            Severity::Warn,
            corr,
            "the written slot was not read back, so nothing here would notice a card \
             that accepted the write and stored something else. ADR-029 §9.2 step 9",
            vec![],
        );
    }

    // --- step 8: the boot half, last ---------------------------------------
    let (written, skipped) = write_boot_half(
        k,
        journal,
        corr,
        plan.boot(),
        &verified,
        request.payload,
        digest,
    )?;

    // --- and now the slot holds this build: retain its header ----------------
    //
    // After the system member and the boot half, and after the read-back when
    // it ran, so the record is written only once the slot is what it names. It
    // does not wait for the arming: the slot holds these bytes whether or not
    // the next boot tries them. **A failure here does not fail the install** —
    // the slot is written and about to be armed — and costs exactly one thing,
    // said in the journal: a re-send of this build will not be recognised as
    // running.
    let record = rivet_core::deploy::slot_header_path(slot_name);
    if let Err(e) = k.write_file_synced(&record, head) {
        journal.emit(
            k,
            Severity::Warn,
            corr,
            "the slot is written and its record could not be, so a later re-send of this build \
             will not be recognised as what this machine runs — PLAN-Feature-Set-2026-09-12 §3",
            vec![
                field("file", Value::Str(record.clone())),
                field("error", Value::Str(e.to_string())),
            ],
        );
    }

    // --- step 10: arm ------------------------------------------------------
    let armed = {
        let mut medium = KernelMedium::new(k);
        let outcome = slots.arm_trial_boot(&mut medium, slot);
        let said = medium.warnings();
        for line in said {
            journal.emit(k, Severity::Warn, corr, &line, vec![]);
        }
        outcome.map_err(arm_refused)?
    };
    journal.emit(
        k,
        Severity::Info,
        corr,
        "the next boot is a trial of the slot just written",
        vec![
            field("target_partition", Value::Int(i64::from(slot.0))),
            field("reboot", Value::Str(armed.to_string())),
            field("system_bytes", count(system.bytes)),
            field("boot_written", Value::Int(written as i64)),
            field("boot_skipped", Value::Int(skipped as i64)),
        ],
    );

    Ok(Outcome::Installed {
        slot,
        armed,
        system_bytes: system.bytes,
        boot_written: written,
        boot_skipped: skipped,
    })
}

// ---------------------------------------------------------------------------
// The steps.
// ---------------------------------------------------------------------------

/// The payload file, as `rivet_core::package::verify` reads an artifact.
///
/// **This is the whole of what this binary contributes to verification** since
/// 2026-09-12: where the bytes come from. The order, the refusals, and which
/// failure is the payload's rather than the card's are the core's, and the
/// artifact store's door in `cmd/rivet-mcpd` asks the same functions over a
/// plain file — one implementation, consulted at two points (`ADR-052`
/// §7). Until that day this was `verify_member`, written here, and the door
/// would have been its second copy.
struct PayloadFile<'a> {
    k: &'a mut dyn Kernel,
    path: &'a str,
}

impl<'a> PayloadFile<'a> {
    fn new(k: &'a mut dyn Kernel, path: &'a str) -> Self {
        Self { k, path }
    }
}

impl verify::Source for PayloadFile<'_> {
    type Error = Errno;

    fn read_range(
        &mut self,
        offset: u64,
        len: u64,
        sink: &mut dyn FnMut(&[u8]) -> bool,
    ) -> Result<(), Errno> {
        self.k.read_range(self.path, offset, len, sink)
    }
}

/// A verification that did not pass, as an [`Outcome`].
///
/// **Two kinds, because they mean opposite things**: the payload's fault is
/// [`Outcome::RefusedPayload`], and a card that would not give up the bytes is
/// [`Outcome::RefusedMachine`] — a different payload would not help it.
fn fault(path: &str, why: Fault<Errno>) -> Outcome {
    match why {
        Fault::Artifact(e) => refused(e),
        Fault::Medium {
            reading: Reading::Header,
            error,
        } => Outcome::RefusedMachine(format!("cannot read the header of {path}: {error}")),
        Fault::Medium {
            reading: Reading::Member(index),
            error,
        } => Outcome::RefusedMachine(format!("cannot read member {index} of {path}: {error}")),
    }
}

/// `ADR-029` §9.2 step 7.
///
/// **Takes a [`VerifiedMember`], and that is the point.** The token cannot be
/// constructed by a caller, so this function is unreachable with bytes whose
/// digest has not matched the signed header — §5 step 8's *"refuse on mismatch,
/// having written nothing further"*, expressed in the type system rather than in
/// a comment.
fn write_whole_partition(
    k: &mut dyn Kernel,
    verified: VerifiedMember<'_>,
    payload: &str,
    device: &str,
) -> Result<u64, Errno> {
    let member = verified.member();
    k.write_device(payload, member.offset(), member.stored_len(), device)
}

/// `ADR-029` §9.2 step 8 — the boot half, skipping every file the target
/// already holds.
///
/// Returns `(written, skipped)`.
///
/// **When the boot half is the partition holding the default-slot record** —
/// slot A's, `p1` — the whole of it runs under
/// `rivet_adapter_image::AUTOBOOT_LOCK`: both mounts, the selector read before
/// and the repair after. The whole, and not each mount, because the repair
/// restores the selector bytes read before the write; a commit or an arming
/// landing between the two would be undone by it. A lock still held after
/// [`crate::medium::LOCK_WAIT_MS`] refuses the install before anything is
/// mounted. Slot B's boot half is another partition and takes no lock.
#[allow(clippy::too_many_arguments)]
fn write_boot_half(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    corr: &str,
    boot: &BootHalf,
    verified: &[VerifiedMember<'_>],
    payload: &str,
    digest: &mut dyn Digest,
) -> Result<(usize, usize), Outcome> {
    let shares_the_record = autoboot_at_risk(boot).is_some();
    if shares_the_record {
        k.lock_within(AUTOBOOT_LOCK, crate::medium::LOCK_WAIT_MS)
            .map_err(|e| {
                Outcome::RefusedMachine(format!(
                    "cannot take {AUTOBOOT_LOCK} to write the boot half on {}, which holds the \
                     default-slot record, so nothing was mounted or written: {e}",
                    boot.device
                ))
            })?;
    }
    let outcome = write_boot_half_held(k, journal, corr, boot, verified, payload, digest);
    if shares_the_record {
        k.unlock(AUTOBOOT_LOCK);
    }
    outcome
}

/// [`write_boot_half`], with whatever lock it needs already held.
#[allow(clippy::too_many_arguments)]
fn write_boot_half_held(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    corr: &str,
    boot: &BootHalf,
    verified: &[VerifiedMember<'_>],
    payload: &str,
    digest: &mut dyn Digest,
) -> Result<(usize, usize), Outcome> {
    // Decide first, mount second. Skip-if-identical is what makes the usual
    // update never open the FAT filesystem for writing at all, and that is a
    // *safety* property rather than an optimisation (`ADR-029` §9, F5) — so the
    // read-only pass is separate from the writable one and a payload that
    // changes nothing never mounts writable.
    //
    // **The two mounts are sequential and the unmount between them is
    // load-bearing.** `Kernel::mount` treats "already mounted here" as the
    // convergent answer, so a *failed* unmount would leave the read-only mount
    // in place and the writable mount would converge onto it — and every write
    // below would then answer `EROFS` on a perfectly healthy card. That is why
    // a failed unmount is a record rather than a silence.
    let mut todo: Vec<&BootWrite> = Vec::new();
    let mut skipped = 0usize;
    let mounted = mount_boot(k, boot, false)?;
    // `ADR-029` §9.3's mitigation begins here, on the READ-ONLY mount, because
    // the bytes to restore have to be the ones from before anything was opened
    // for writing. Reading it costs nothing on the usual update, where `todo`
    // turns out to be empty and the filesystem is never made writable at all.
    let guarded = autoboot_at_risk(boot);
    let selector_before = guarded
        .as_ref()
        .and_then(|path| k.read_bytes(path).ok());
    for file in &boot.files {
        let path = boot.path_of(PAYLOAD_BOOT_MOUNT, &file.name);
        let found = k.read_bytes(&path).ok().map(|bytes| digest.of(&bytes));
        match action_for(&file.digest, found.as_ref()) {
            BootFileAction::Skip => skipped += 1,
            BootFileAction::Write => todo.push(file),
        }
    }
    if let Some(path) = guarded.as_ref()
        && selector_before.is_none()
    {
        // Loud, because the alternative is repairing nothing later and saying
        // nothing about why. A missing selector is `ADR-029` §9.3's failure
        // already in place rather than a risk of it.
        journal.emit(
            k,
            Severity::Warn,
            corr,
            "the boot selector could not be read before writing the boot half, so this install \
             cannot detect or repair damage to it — ADR-029 §9.3. The firmware falls back to \
             partition 1 when it will not parse, and during an update that is the slot being \
             written",
            vec![field("file", Value::Str(path.clone()))],
        );
    }
    unmount_boot(k, journal, corr, mounted);

    if todo.is_empty() {
        journal.emit(
            k,
            Severity::Info,
            corr,
            "the boot half already holds every file this payload carries, so it is not \
             opened for writing at all",
            vec![field("skipped", Value::Int(skipped as i64))],
        );
        return Ok((0, skipped));
    }

    let mounted = mount_boot(k, boot, true)?;
    let mut written = 0usize;
    for file in &todo {
        let token = verified
            .iter()
            .copied()
            .find(|v| v.member().index() == file.member_index)
            .ok_or_else(|| Outcome::RefusedMachine(format!("{} was never verified", file.name)))?;
        let outcome = write_boot_file(k, token, payload, boot, &file.name);
        if let Err(e) = outcome {
            unmount_boot(k, journal, corr, mounted);
            return Err(Outcome::RefusedMachine(format!(
                "cannot write {} onto {}: {e}. The slot is left as it is — ADR-029 F3",
                file.name, boot.device
            )));
        }
        written += 1;
        journal.emit(
            k,
            Severity::Info,
            corr,
            "a boot-half file is written",
            vec![
                field("name", Value::Str(file.name.clone())),
                field("bytes", count(file.bytes)),
            ],
        );
    }
    // One `sync(2)` for the whole half, after every rename. The per-file
    // `fsync` is in `write_file_synced`; this is the directory-level flush that
    // an unjournalled FAT filesystem needs before the mount goes away.
    k.sync();
    // `ADR-029` §9.3, and it is AFTER that sync deliberately: the question this
    // answers is whether the selector survived the writes reaching the card, so
    // asking before the flush would be reading the page cache and getting the
    // reassuring answer every time.
    if let (Some(path), Some(before)) = (guarded.as_ref(), selector_before.as_ref()) {
        repair_autoboot(k, journal, corr, path, before);
    }
    unmount_boot(k, journal, corr, mounted);
    Ok((written, skipped))
}

/// **Where `autoboot.txt` is at risk from this write, if it is at all.**
///
/// `ADR-029` §9.3's F5 hole, and the first half of the mitigation the maintainer
/// ruled on 2026-08-29 (`PLAN.md` §94.2): **detect and repair.**
///
/// # What the hole is
///
/// §9.3 settled it on hardware rather than by reasoning: the firmware obeys
/// `autoboot.txt` when it can and **falls back to partition 1 when it cannot**.
/// So writing slot A's boot half — which *is* partition 1 — puts the selector
/// on the same FAT filesystem as the kernels being written, and a torn write
/// that damages the directory takes `autoboot.txt` with it. The next boot then
/// does not stop and does not stay on the running slot: it boots partition 1,
/// **which is the half-written slot**. None of §9's three existing bounds
/// reaches it.
///
/// # Why this returns `None` for slot B
///
/// The autoboot partition is always the first one (`ADR-018` §5.1), so an
/// update targeting slot B writes a *different* filesystem and cannot damage
/// the selector at all. Comparing devices rather than slot names keeps that
/// true on a machine whose layout is not the Pi's — the boot-slot port (x86 has no `tryboot`; ruled 2026-08-23)
/// expects `/EFI/rivet/a/` on a UEFI board, where this whole mechanism does not
/// apply and the comparison simply never matches.
///
/// # It is the mount root and not [`BootHalf::path_of`]
///
/// `autoboot.txt` lives at the partition root because that is where the
/// firmware reads it, not inside whatever directory this slot's boot half
/// occupies. Using `path_of` here would look for it under `/EFI/rivet/a/` on
/// the machine where this returns `None` anyway, and would be wrong the day it
/// did not.
fn autoboot_at_risk(boot: &BootHalf) -> Option<String> {
    (boot.device == autoboot_partition_device())
        .then(|| format!("{PAYLOAD_BOOT_MOUNT}/{AUTOBOOT_FILE}"))
}

/// **Restore `autoboot.txt` if writing the boot half changed it** — the repair
/// half of `ADR-029` §9.3's mitigation.
///
/// # Why this compares bytes rather than parsing
///
/// Because the installer is **never** entitled to change this file, so any
/// difference is damage by definition. `ADR-029` §6 makes `autoboot.txt` a
/// reserved member name and step 5 refuses a whole payload that carries one —
/// there is a test named for it. The selector is written by *arming*
/// (`ADR-029` §9.1), which is a separate operation that happens after this one.
///
/// That makes byte equality both the simplest check and a **strictly stronger**
/// one than re-parsing. A parse check passes a file whose `boot_partition=2`
/// became `boot_partition=1` under a single flipped byte — which is a card that
/// silently boots the wrong slot, the exact failure §9.3 is about. Byte
/// equality catches that; parsing does not.
///
/// # What it does not do, stated plainly
///
/// **This narrows the window; it does not close it.** §9.3 says so of this
/// candidate and it is worth repeating where the code is: power lost between
/// the damaging write and this repair leaves the selector damaged, and the next
/// boot is then the one §9.3 describes. Closing the gap needs §9.3's second
/// candidate — refusing to write slot A's boot half from a machine running slot
/// B — which is not ruled and is not this.
///
/// # A file that was already unparseable is reported and NOT rewritten
///
/// If it arrived damaged, this install did not do it, and restoring bytes that
/// were themselves broken would launder a pre-existing fault into a repair
/// record. `AGENTS.md` §5: report what the system said. A board in that state is
/// one reboot from a surprise and the operator needs to know, which is why it is
/// a `warn` and not a silence.
fn repair_autoboot(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    corr: &str,
    path: &str,
    before: &[u8],
) {
    let after = k.read_bytes(path).ok();
    if after.as_deref() == Some(before) {
        journal.emit(
            k,
            Severity::Debug,
            corr,
            "the boot selector reads back byte-for-byte as it did before the boot half was \
             written, so this update did not disturb it — ADR-029 §9.3",
            vec![field("file", Value::Str(path.to_string()))],
        );
        return;
    }

    // Write-sync-rename, for the reason `write_boot_file` uses it: the firmware
    // must never see this name half-written, and a repair that could itself
    // tear would be the hole it is closing.
    let temp = format!("{path}.new");
    let restored = k
        .write_file_synced(&temp, before)
        .and_then(|()| k.rename(&temp, path));
    k.sync();

    let verified = k.read_bytes(path).ok().as_deref() == Some(before);
    journal.emit(
        k,
        Severity::Warn,
        corr,
        if restored.is_ok() && verified {
            "THE BOOT SELECTOR WAS DAMAGED BY WRITING THE BOOT HALF AND HAS BEEN RESTORED from \
             the bytes read before the write. This is ADR-029 §9.3's F5 window being hit rather \
             than a routine event: an unrepaired autoboot.txt boots partition 1, which during an \
             update IS the half-written slot. The card should be treated as suspect"
        } else {
            "THE BOOT SELECTOR WAS DAMAGED BY WRITING THE BOOT HALF AND THE REPAIR DID NOT TAKE. \
             The firmware falls back to partition 1 when autoboot.txt will not parse, and during \
             an update that is the half-written slot — ADR-029 §9.3. DO NOT REBOOT THIS MACHINE \
             until the file is corrected; ADR-035 §8A.2's boot partition is reachable from any \
             machine with a card reader"
        },
        vec![
            field("file", Value::Str(path.to_string())),
            field("restored", Value::Int(i64::from(restored.is_ok() && verified))),
            field("bytes", count(before.len() as u64)),
        ],
    );
}

/// One boot-half file, write-sync-rename.
///
/// `ADR-029` §9's third bound on F5, and the pattern `write_autoboot` already
/// uses on the same filesystem: the firmware must never see a file under its own
/// name that is half a kernel.
fn write_boot_file(
    k: &mut dyn Kernel,
    verified: VerifiedMember<'_>,
    payload: &str,
    boot: &BootHalf,
    name: &str,
) -> Result<(), Errno> {
    let member = verified.member();
    let mut bytes = Vec::with_capacity(usize::try_from(member.stored_len()).unwrap_or(0));
    read_into_vec(k, payload, member.offset(), member.stored_len(), &mut bytes)?;
    let temp = boot.path_of(PAYLOAD_BOOT_MOUNT, &BootHalf::temp_name(name));
    let final_path = boot.path_of(PAYLOAD_BOOT_MOUNT, name);
    k.write_file_synced(&temp, &bytes)?;
    k.rename(&temp, &final_path)
}

/// Mount the boot half at [`PAYLOAD_BOOT_MOUNT`], saying whether this call
/// made the mount or found one there. [`unmount_boot`] releases only a mount
/// this call made.
fn mount_boot(k: &mut dyn Kernel, boot: &BootHalf, writable: bool) -> Result<Mounted, Outcome> {
    k.mount(&boot.device, PAYLOAD_BOOT_MOUNT, boot.fstype, writable)
        .map_err(|e| {
            Outcome::RefusedMachine(format!(
                "cannot mount {} on {PAYLOAD_BOOT_MOUNT}: {e}",
                boot.device
            ))
        })
}

/// Release what [`mount_boot`] returned: unmount a mount this install made,
/// and leave one it found in place — somebody else's, which it read and wrote
/// through. The found one is journalled, because a mount nobody unmounts is
/// worth a record.
fn unmount_boot(k: &mut dyn Kernel, journal: &mut Journal, corr: &str, mounted: Mounted) {
    if mounted == Mounted::Adopted {
        journal.emit(
            k,
            Severity::Warn,
            corr,
            &format!(
                "{PAYLOAD_BOOT_MOUNT} was already mounted when this install mounted it, so the \
                 install used that mount and left it mounted"
            ),
            vec![],
        );
        return;
    }
    if let Err(e) = k.umount(PAYLOAD_BOOT_MOUNT) {
        journal.emit(
            k,
            Severity::Warn,
            corr,
            &format!("could not unmount {PAYLOAD_BOOT_MOUNT}: {e}"),
            vec![],
        );
    }
}

// ---------------------------------------------------------------------------
// Small helpers over the port.
// ---------------------------------------------------------------------------

fn read_into_vec(
    k: &mut dyn Kernel,
    path: &str,
    offset: u64,
    len: u64,
    into: &mut Vec<u8>,
) -> Result<(), Errno> {
    k.read_range(path, offset, len, &mut |chunk| {
        into.extend_from_slice(chunk);
        true
    })
}

fn digest_of_range(
    k: &mut dyn Kernel,
    path: &str,
    offset: u64,
    len: u64,
    digest: &mut dyn Digest,
) -> Result<[u8; 32], Errno> {
    digest.start();
    let mut fed = 0u64;
    k.read_range(path, offset, len, &mut |chunk| {
        digest.write(chunk);
        fed += chunk.len() as u64;
        true
    })?;
    if fed != len {
        return Err(Errno::EIO);
    }
    Ok(digest.finish())
}

fn refused(e: PackageError) -> Outcome {
    Outcome::RefusedPayload(e.to_string())
}

/// Both kinds of arming refusal are the *machine's* rather than the payload's —
/// a different `.rvt` would change neither — but the two are not the same thing
/// and the message says which, because `ArmRefusal` exists precisely so an
/// operator can tell "retrying will not help" from "look at the hardware"
/// (`ADR-006` §4.1).
fn arm_refused(e: ArmRefusal) -> Outcome {
    Outcome::RefusedMachine(match e {
        ArmRefusal::Unsafe(why) => format!("the arming is refused as unsafe: {why}"),
        ArmRefusal::Medium(why) => format!("the card or the kernel refused the arming: {why}"),
    })
}

/// **Empty `slot`'s retained-header record**, creating its directory if this is
/// the first install to write that slot. `rivet_core::deploy::SLOT_HEADERS`.
///
/// Empty rather than removed because the port has no removal of a regular file
/// and an empty record names nothing: `rivet_core::deploy::already_running`
/// matches only a whole header.
fn forget_what_the_slot_holds(
    k: &mut dyn Kernel,
    slot: rivet_core::state::Slot,
) -> Result<(), Outcome> {
    let cannot = |what: &str, e: Errno| {
        Outcome::RefusedMachine(format!(
            "cannot {what}: {e}. NOTHING WAS WRITTEN to the slot. The record of what a slot holds \
             is emptied before the slot is written so that it can never name a build the slot no \
             longer holds (PLAN-Feature-Set-2026-09-12 §3); /state refusing that small write \
             is a /state that will not take an install either"
        ))
    };
    let dir = rivet_core::deploy::slot_header_dir(slot);
    k.mkdir_mode(rivet_core::deploy::SLOT_HEADERS, rivet_core::package::install::DIR_MODE)
        .map_err(|e| cannot(&format!("create {}", rivet_core::deploy::SLOT_HEADERS), e))?;
    k.mkdir_mode(&dir, rivet_core::package::install::DIR_MODE)
        .map_err(|e| cannot(&format!("create {dir}"), e))?;
    let record = rivet_core::deploy::slot_header_path(slot);
    k.write_file_synced(&record, &[])
        .map_err(|e| cannot(&format!("empty {record}"), e))
}

/// **The refusal for a system update while this boot is not committed**, or
/// `None` when the update may proceed. `ADR-018` §5.
///
/// # The condition, and why it is not the tryboot flag
///
/// The hazard is precise: **the slot an update writes is the slot an ordinary
/// reset boots.** An update writes the slot that is not running
/// ([`target_slot`]), and a machine is committed exactly when the default slot
/// is the running one (`rivet_core::operation::machine::SlotRead::committed`).
/// So the update writes the default slot exactly when the boot is not
/// committed, and that is the comparison made.
///
/// `in_trial_boot` is deliberately not consulted. It says how this boot was
/// *entered* and stays true for the life of a trial PID 1 has since confirmed —
/// three readers were misled by it before `default_slot` existed — so refusing
/// on it would refuse every committed trial, and not refusing on it would miss a
/// default that points elsewhere for any other reason.
///
/// # A default that cannot be read is a refusal
///
/// Whether the write would land on the slot a reset boots is then unknown, and
/// unknown is not safe: the one case this guards is a card a reset leaves
/// unbootable.
fn uncommitted_trial(
    running: SlotId,
    target: SlotId,
    default: Result<SlotId, String>,
) -> Option<String> {
    let word = |s: SlotId| slot_named(s).map_or_else(|_| format!("partition {}", s.0), |n| {
        format!("slot {}", n.as_str())
    });
    match default {
        Ok(default) if default == running => None,
        Ok(default) => Some(format!(
            "REFUSED: this boot is NOT COMMITTED. This machine is running {running_word}, and an \
             ordinary reset boots {default_word} — which is what an uncommitted one-shot trial \
             looks like (ADR-018 §5). This update would write {target_word}, the slot that reset \
             falls back to, while the trial is undecided: a reset or a power cut part-way through \
             that write boots a half-written system, and a card is what recovers that. NOTHING WAS \
             WRITTEN. Either let the trial commit — PID 1 confirms a healthy boot, and `rivet get \
             slot` then says `committed: true` — or reset to roll back to {default_word}; then \
             install again. ADR-018 §5.",
            running_word = word(running),
            default_word = word(default),
            target_word = word(target),
        )),
        Err(why) => Some(format!(
            "REFUSED: which slot an ordinary reset boots could not be read ({why}), so whether this \
             update would write the slot a reset falls back to is UNKNOWN. During an uncommitted \
             trial it would, and a reset part-way through that write boots a half-written system. \
             NOTHING WAS WRITTEN. `rivet get slot` reports `default_unmeasured` with the same \
             reason. ADR-018 §5."
        )),
    }
}

/// **Install an application.** `ADR-015` §7, and the other half of
/// `rivet_core::package::install`.
///
/// # Why this is in the binary named `rivet-deploy`
///
/// Because `ADR-029` §11 put all three artifact kinds in **one container**, on
/// the grounds that *"three formats means three verifiers each with a third of
/// the scrutiny"*, and the same sentence is true of installers. Everything up to
/// this point — reading the header, finding the key in the machine's trust
/// store, checking the signature, checking that the key is entitled to sign this
/// *kind* — is identical for an update and an application, and it is the code
/// `ADR-028` §2 calls the security boundary. A second binary would either
/// duplicate it or link it, and the duplicate is the one that eventually
/// diverges.
///
/// **Named `rivet-deploy` since 2026-08-24** — ruled by the maintainer,
/// `PLAN.md` §0.4.6 and `ADR-027` §9.3. It was `rivet-update`, which described
/// half of what it does, and neither `update` nor `install` is the umbrella:
/// *deploy* already was, because the maintainer's 2026-08-24 ruling and §0.8 had been calling this
/// whole path **the deploy chain** since before the binary existed. The rename
/// was taken while it was still free — no service definition named the binary
/// yet, so nothing had to be migrated.
///
/// # Nothing is written before everything is verified
///
/// Every member's digest is checked against the header **before the first byte
/// reaches the filesystem**, which is `ADR-029` §5 step 8 and the property the
/// whole container exists to give. It costs reading each member twice — once to
/// verify, once to write — and for an application that is a few megabytes.
///
/// The alternative, verify-as-you-write, would leave a half-written program on
/// `/state` when a member turned out to be wrong. It would then have to be
/// removed, and a removal that fails is how a machine ends up with a binary
/// nobody signed.
fn application(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    corr: &str,
    request: &Request<'_>,
    package: &Package,
    // The payload's own signed header, retained beside the install so that
    // `ADR-015` §4.2 has something to verify at launch. Passed in rather than
    // re-read from the file: this is the exact buffer the signature was checked
    // over, so what lands on `/state` is provably the bytes that verified.
    head: &[u8],
    digest: &mut dyn Digest,
) -> Result<Outcome, Outcome> {
    // **The built-in volume, and that is not yet a choice.** `ADR-031` §8 item 1
    // made the application root a function of a volume root; nothing chooses one
    // yet, because choosing needs the attached set and `ADR-031` §11 items 1, 2
    // and 8 are unruled — which process attaches, whether a running machine can
    // gain a volume, and where a volume is mounted. Until they are, an install
    // lands where every install has landed.
    let plan =
        rivet_core::package::install::plan(rivet_core::service::BUILT_IN_VOLUME, package)
            .map_err(refused)?;

    journal.emit(
        k,
        Severity::Info,
        corr,
        "this payload is an application, and these are the files it becomes",
        vec![
            field("app_name", Value::Str(plan.name().to_string())),
            field("app_version", Value::Str(plan.version().to_string())),
            field("version_dir", Value::Str(plan.version_dir().to_string())),
            field("steps", Value::Int(plan.steps().len() as i64)),
        ],
    );

    // **Not what `current` already selects** — `PLAN-Feature-Set-2026-09-12`
    // §3. The occupied-directory refusal below would stop it too, and says
    // the wrong thing: that a version directory is in the way, when the truth
    // is that this machine is already running these exact bytes and there is
    // nothing to do. The retained header beside `current` is what that
    // application IS (`ADR-015` §4 re-verifies it at every launch), and it is
    // compared with the header that just verified — `rivet_core::deploy`
    // argues why that is identity and not a second reader.
    let current_header =
        rivet_core::deploy::running_application_header_path(rivet_core::service::BUILT_IN_VOLUME, plan.name());
    if let Ok(retained) = k.read_bytes(&current_header) {
        let retained = [(
            rivet_core::deploy::Running::Application(plan.name().to_string()),
            retained,
        )];
        if let Some(what) = rivet_core::deploy::already_running(head, &retained) {
            let refusal = rivet_core::deploy::already_running_refusal(request.payload, what);
            journal.emit(
                k,
                Severity::Warn,
                corr,
                &refusal,
                vec![
                    field("app_name", Value::Str(plan.name().to_string())),
                    field("app_version", Value::Str(plan.version().to_string())),
                ],
            );
            return Ok(Outcome::RefusedPayload(refusal));
        }
    }

    // **Nothing is unpacked over a directory that is already there.**
    // `ADR-015` §7.2, and the decision is `Plan::occupied`'s rather than this
    // function's — what happens here is the one fact the core cannot have, which
    // is whether the path is on the filesystem.
    //
    // **Before the verification loop and before the dry-run branch**, so that a
    // dry run reports the refusal too: a `--dry-run` answering "this would work"
    // about an install that would be refused is worse than no dry run, and it is
    // the answer an operator plans against.
    //
    // `RefusedPayload` rather than `RefusedMachine`, and the choice is about
    // `Outcome::wrote_nothing`: nothing reached the filesystem and that is
    // provable here, which is the contract of that variant. It is also literally
    // true that a different payload would help — one whose version differs
    // installs, which is what `ADR-027` §3.2.2 makes a rebuild produce.
    if let Some(refusal) = plan.occupied(k.path_exists(plan.version_dir())) {
        journal.emit(
            k,
            Severity::Warn,
            corr,
            &refusal,
            vec![
                field("app_name", Value::Str(plan.name().to_string())),
                field("app_version", Value::Str(plan.version().to_string())),
                field("version_dir", Value::Str(plan.version_dir().to_string())),
            ],
        );
        return Ok(Outcome::RefusedPayload(refusal));
    }

    // --- verify every member, before anything is written --------------------
    verify::members(&mut PayloadFile::new(k, request.payload), package, digest)
        .map_err(|why| fault(request.payload, why))?;
    journal.emit(
        k,
        Severity::Info,
        corr,
        "every member's bytes are the ones the signature covers",
        vec![field("members", Value::Int(package.members().len() as i64))],
    );

    // --- Shared Rivets: the declared closure, checked HERE ------------------
    //
    // *"The installer refuses an application whose closure is not satisfied,
    // naming what is missing — refusal at install, never a loader error at
    // start."* This is that sentence.
    //
    // **After the members verify and before anything is written**, and the
    // order is the argument: the manifest is a member, so reading it before it
    // verified would be trusting bytes the signature had not yet vouched for —
    // and refusing after a write would leave an application on the machine that
    // cannot run.
    if let Some(refusal) = closure_refusal(k, package, request, digest)? {
        journal.emit(k, Severity::Warn, corr, &refusal, vec![]);
        return Ok(Outcome::RefusedPayload(refusal));
    }

    if request.dry_run {
        for step in plan.steps() {
            journal.emit(
                k,
                Severity::Info,
                corr,
                "PLANNED",
                vec![field("step", Value::Str(describe(step)))],
            );
        }
        return Ok(Outcome::PlannedApplication {
            name: plan.name().to_string(),
            version: plan.version().to_string(),
            steps: plan.steps().len(),
        });
    }

    // --- perform, in the plan's order ---------------------------------------
    let files = perform(k, request, package, head, plan.steps())?;
    k.sync();

    journal.emit(
        k,
        Severity::Info,
        corr,
        "the application is installed and is what `current` points at",
        vec![
            field("app_name", Value::Str(plan.name().to_string())),
            field("app_version", Value::Str(plan.version().to_string())),
            field("files", Value::Int(files as i64)),
            field(
                "next",
                Value::Str(
                    "the supervisor starts it on the next convergence; nothing here starts it"
                        .to_string(),
                ),
            ),
        ],
    );

    Ok(Outcome::InstalledApplication {
        name: plan.name().to_string(),
        version: plan.version().to_string(),
        files,
    })
}

/// The refusal for an application whose declared closure this machine cannot
/// satisfy, or `None` if it can. Shared Rivets, 2026-08-31.
///
/// # Why it re-reads and re-verifies the manifest
///
/// [`application`] has already streamed every member past a digest, but it kept
/// none of the bytes — it is checking, not reading. This reads the manifest
/// member a second time and takes the [`VerifiedMember`] token again, which
/// costs a few kilobytes and buys the property that matters: the closure this
/// function acts on is one the signature covered, and there is no path through
/// this file where an unverified manifest reaches a decision.
///
/// # An empty closure is the common case and costs nothing
///
/// Every application built before 2026-08-31 declares none, and so does every
/// statically-linked one after. The loop below runs zero times for them.
fn closure_refusal(
    k: &mut dyn Kernel,
    package: &Package,
    request: &Request<'_>,
    digest: &mut dyn Digest,
) -> Result<Option<String>, Outcome> {
    let application = rivet_core::package::application::Application::open(package)
        .map_err(refused)?;
    let index = application.manifest_member().index();

    let mut bytes = Vec::new();
    read_into_vec(
        k,
        request.payload,
        application.manifest_member().offset(),
        application.manifest_member().stored_len(),
        &mut bytes,
    )
    .map_err(|e| {
        Outcome::RefusedMachine(format!(
            "cannot re-read the manifest of {}: {e}",
            request.payload
        ))
    })?;

    let verified = verify::member(&mut PayloadFile::new(k, request.payload), package, index, digest)
        .map_err(|why| fault(request.payload, why))?;
    let manifest = application
        .read_manifest(verified, &bytes)
        .map_err(refused)?;

    let closure = manifest.library();
    if closure.is_empty() {
        return Ok(None);
    }

    let volume = rivet_core::service::BUILT_IN_VOLUME;
    let missing =
        rivet_core::package::install::unsatisfied(volume, closure, |path| k.path_exists(path));
    if missing.is_empty() {
        return Ok(None);
    }

    Ok(Some(format!(
        "REFUSED: this application declares {} librar{} and {} of them {} not on this machine. \
         NOTHING WAS WRITTEN.\n\
         \n\
         Missing:\n{}\n\
         \n\
         Each of those is a signed library .rvt that installs to {volume}/{}/<the same string>. \
         Install them first — `cargo rivet library` emitted them and printed exactly these \
         references — then install this application again.\n\
         \n\
         This is refused HERE rather than at start on purpose. An application installed \
         without its libraries would be a service that will not run, whose only account is a \
         loader saying it cannot find a file; this names the artifact to go and fetch.",
        closure.len(),
        if closure.len() == 1 { "y" } else { "ies" },
        missing.len(),
        if missing.len() == 1 { "is" } else { "are" },
        missing
            .iter()
            .map(|r| format!("  {r}"))
            .collect::<Vec<_>>()
            .join("\n"),
        rivet_core::package::library::LIB_DIR,
    )))
}

/// Install a **library** artifact. Shared Rivets, 2026-08-31.
///
/// Structurally the same as [`application`] and shorter, because a library has
/// no `current` link, no service name, no manifest of permissions to grant and
/// nothing that starts. What it has instead is a **content-addressed
/// directory**, and the two consequences of that are the only places this
/// function is interesting:
///
/// 1. **The reference is computed from the header that verified**, not from the
///    filename and not from a field. `head` is the exact buffer the signature
///    was checked over, so the digest in the directory name provably describes
///    the artifact this machine accepted.
/// 2. **A directory whose retained header is already there is a success.**
///    See [`Outcome::LibraryAlreadyInstalled`] and
///    `rivet_core::package::install::LibraryPlan::already_installed` for why
///    the header, and never the directory alone, is what this asks about.
fn library(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    corr: &str,
    request: &Request<'_>,
    package: &Package,
    head: &[u8],
    digest: &mut dyn Digest,
) -> Result<Outcome, Outcome> {
    // The identity, over the same bytes the signature covered. Computed here,
    // in the same function that plans the install, so that the digest and the
    // package cannot come apart — `install::library_plan` documents why it
    // takes this as a parameter it cannot check.
    let identity = rivet_core::package::library::identity(head, digest).ok_or_else(|| {
        Outcome::RefusedPayload(
            "this payload's header is too short to have an identity, which cannot happen for a \
             package that verified — so this is a defect in rivet-deploy rather than in the file"
                .to_string(),
        )
    })?;

    let plan = rivet_core::package::install::library_plan(
        rivet_core::service::BUILT_IN_VOLUME,
        package,
        &identity,
    )
    .map_err(refused)?;

    journal.emit(
        k,
        Severity::Info,
        corr,
        "this payload is a library, and this is the directory it becomes",
        vec![
            field("reference", Value::Str(plan.reference().to_string())),
            field("library_dir", Value::Str(plan.library_dir().to_string())),
            field("object_dir", Value::Str(plan.object_dir().to_string())),
            field("steps", Value::Int(plan.steps().len() as i64)),
        ],
    );

    if plan.already_installed(k.path_exists(plan.header_path())) {
        let reference = plan.reference().to_string();
        journal.emit(
            k,
            Severity::Info,
            corr,
            "this library is already on this machine, byte for byte, so nothing was written",
            vec![
                field("reference", Value::Str(reference.clone())),
                field("library_dir", Value::Str(plan.library_dir().to_string())),
                field(
                    "why",
                    Value::Str(
                        "a library directory is named by a digest of its own contents, and this \
                         one's retained header — which this installer writes last, after every \
                         object and the manifest — is already on disk, so every byte it \
                         describes finished writing"
                            .to_string(),
                    ),
                ),
            ],
        );
        return Ok(Outcome::LibraryAlreadyInstalled { reference });
    }

    verify::members(&mut PayloadFile::new(k, request.payload), package, digest)
        .map_err(|why| fault(request.payload, why))?;
    journal.emit(
        k,
        Severity::Info,
        corr,
        "every member's bytes are the ones the signature covers",
        vec![field("members", Value::Int(package.members().len() as i64))],
    );

    if request.dry_run {
        for step in plan.steps() {
            journal.emit(
                k,
                Severity::Info,
                corr,
                "PLANNED",
                vec![field("step", Value::Str(describe(step)))],
            );
        }
        return Ok(Outcome::PlannedLibrary {
            reference: plan.reference().to_string(),
            steps: plan.steps().len(),
        });
    }

    let files = perform(k, request, package, head, plan.steps())?;
    k.sync();

    journal.emit(
        k,
        Severity::Info,
        corr,
        "the library is installed",
        vec![
            field("reference", Value::Str(plan.reference().to_string())),
            field("object_dir", Value::Str(plan.object_dir().to_string())),
            field("files", Value::Int(files as i64)),
            field(
                "next",
                Value::Str(
                    "nothing runs a library. An application that declares this reference in its \
                     closure can now be installed; one already installed picks it up at its next \
                     start"
                        .to_string(),
                ),
            ),
        ],
    );

    Ok(Outcome::InstalledLibrary {
        reference: plan.reference().to_string(),
        files,
    })
}

/// Carry out an ordered list of [`Step`]s, returning how many files were
/// written.
///
/// **Extracted 2026-08-31**, when the library kind needed the same executor.
/// It is byte-for-byte the loop [`application`] had, and pulling it out rather
/// than writing a second one is `ADR-029` §11's argument at the level below the
/// verifier: two executors for one plan type is two places for a mode or an
/// ordering to be wrong.
///
/// **`bytes_so_far` is summed here, not by a second walk** — `R34`,
/// `.rulings/` R-163. This loop already reads every member's bytes to write
/// them and already has the header's length in hand, so [`Step::Size`] spends
/// nothing beyond a running total and one more small write; it is what makes
/// [`Step::Size`]'s own write correct, by construction, rather than a figure
/// this function has to trust from somewhere else.
fn perform(
    k: &mut dyn Kernel,
    request: &Request<'_>,
    package: &Package,
    head: &[u8],
    steps: &[Step],
) -> Result<usize, Outcome> {
    let mut files = 0usize;
    let mut bytes_so_far: u64 = 0;
    for step in steps {
        match step {
            Step::Directory { path, mode } => {
                k.mkdir_mode(path, *mode)
                    .map_err(|e| Outcome::RefusedMachine(format!("cannot create {path}: {e}")))?;
            }
            Step::Member { index, path, mode } => {
                let member = package.members().get(*index).ok_or_else(|| {
                    Outcome::RefusedMachine(format!(
                        "the plan names member {index}, which is not in this package"
                    ))
                })?;
                let mut bytes = Vec::with_capacity(member.stored_len() as usize);
                read_into_vec(
                    k,
                    request.payload,
                    member.offset(),
                    member.stored_len(),
                    &mut bytes,
                )
                .map_err(|e| {
                    Outcome::RefusedMachine(format!(
                        "cannot re-read member {index} of {}: {e}",
                        request.payload
                    ))
                })?;
                k.write_file_synced(path, &bytes)
                    .map_err(|e| Outcome::RefusedMachine(format!("cannot write {path}: {e}")))?;
                // The mode is set after the write rather than at creation,
                // because a file created 0555 cannot then be written to.
                k.chmod(path, *mode).map_err(|e| {
                    Outcome::RefusedMachine(format!("cannot set the mode of {path}: {e}"))
                })?;
                files += 1;
                bytes_so_far += bytes.len() as u64;
            }
            Step::Header { path, mode } => {
                // No digest check and none is possible: the header is what
                // every other digest is *in*, and its own integrity is the
                // signature that was checked at step 1. These are those bytes.
                k.write_file_synced(path, head).map_err(|e| {
                    Outcome::RefusedMachine(format!("cannot write the signed header {path}: {e}"))
                })?;
                k.chmod(path, *mode).map_err(|e| {
                    Outcome::RefusedMachine(format!("cannot set the mode of {path}: {e}"))
                })?;
                files += 1;
                bytes_so_far += head.len() as u64;
            }
            Step::Size { path } => {
                // **The sum of every member and header step above, and
                // nothing else** — not this file's own bytes, which is why
                // this step comes last: a directory holding one more entry
                // than the number it names would be a contradiction if this
                // counted itself.
                let record = format!("{bytes_so_far}\n");
                k.write_file_synced(path, record.as_bytes()).map_err(|e| {
                    Outcome::RefusedMachine(format!("cannot record the size at {path}: {e}"))
                })?;
                k.chmod(path, FILE_MODE).map_err(|e| {
                    Outcome::RefusedMachine(format!("cannot set the mode of {path}: {e}"))
                })?;
                files += 1;
            }
            Step::Activate {
                link,
                staged,
                target,
            } => {
                // A staging name left behind by an install that died between
                // these two calls would make `symlink` fail with EEXIST for
                // ever. Removing it is the one deletion this performs, and it
                // can only ever remove a link this code made.
                let _ = k.unlink_symlink(staged);
                k.symlink(target, staged).map_err(|e| {
                    Outcome::RefusedMachine(format!("cannot stage the link at {staged}: {e}"))
                })?;
                k.rename(staged, link)
                    .map_err(|e| Outcome::RefusedMachine(format!("cannot activate {link}: {e}")))?;
            }
        }
    }
    Ok(files)
}

/// One step, in the words a person reads in a dry run.
fn describe(step: &Step) -> String {
    match step {
        Step::Directory { path, mode } => format!("create {path} mode {mode:04o}"),
        Step::Member { path, mode, .. } => format!("write  {path} mode {mode:04o}"),
        Step::Header { path, mode } => format!("header {path} mode {mode:04o}"),
        Step::Size { path } => format!("size   {path} — the sum of every member and the header above"),
        Step::Activate { link, target, .. } => format!("point  {link} at {target}"),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    //! The install sequence, end to end, against a machine that is a
    //! `BTreeMap`.
    //!
    //! # What is real here and what is not, before anything else
    //!
    //! **The cryptography is real.** These tests build an actual `.rvt` with
    //! `rivet_core::package::write::Builder`, digest every member with
    //! **BLAKE3** and sign the header with **Ed25519**, both out of
    //! `rivet-crypto` — and the installer verifies them with the same code a
    //! board would run. Nothing here is a toy digest agreeing with itself.
    //!
    //! **The machine is not.** `crate::testkernel` is a `BTreeMap`, so what
    //! these prove is that the *sequence* is right — the order of the writes,
    //! that nothing is written before every member verifies, that a refusal
    //! leaves the slot byte-identical, and that the arming is last. `AGENTS.md`
    //! §6: that is not "an update works". `ADR-026` §4 puts `/dev/mmcblk0` in
    //! the column QEMU cannot reach at all, and a card is the only thing that
    //! settles the SD controller, erase blocks, and a card that goes read-only
    //! mid-write.
    //!
    //! The one hazard the fake **can** model, and a loop device cannot, is
    //! `ADR-029` §9.2 step 9's: a device that accepts a write and stores
    //! something else. `FakeKernel::make_it_lie` is that, and
    //! `a_card_that_stores_something_else_is_caught_by_the_read_back` is the
    //! test the step exists for.

    use super::*;
    use rivet_adapter_image::{
        AUTOBOOT_PARTITION_MOUNT, BOOT_DISK, DT_BOOT_PARTITION, PiFirmware,
    };
    // Used only by the tests below, which is why a non-test `cargo check`
    // reported the crate-level import as unused. Scoped here rather than
    // deleted: `application.rs` enforces this name on the member, and these
    // tests build the member that has to satisfy it.
    use rivet_core::package::application::MANIFEST_NAME;
    use rivet_core::package::format::HEADER_BYTES;
    use rivet_core::package::kind::Kind;
    use rivet_core::package::port::Sign;
    use rivet_core::package::update::{SlotVariance, TargetClass};
    use rivet_core::package::write::{Builder, NewMember};
    use rivet_crypto::{Blake3, Ed25519Signer, Ed25519Verifier, SEED_BYTES};

    use crate::testkernel::{FakeKernel, Script};

    /// Small enough that a test runs in microseconds, large enough that the
    /// fake's chunking loop goes round more than once.
    const SYSTEM_BYTES: usize = 200 * 1024;

    const KEY_PATH: &str = "/system/keys/root.pub";
    const PAYLOAD_PATH: &str = "/state/rivet-0.0.2-aarch64-rpi4-update.rvt";

    /// Minted here, in memory, never written down — `AGENTS.md` §8.3.
    fn signer() -> Ed25519Signer {
        let mut digest = Blake3::new();
        // The literal is a DOMAIN SEPARATOR, not a name, and it is deliberately
        // not renamed with the binary: its only requirement is to stay stable,
        // because the key it derives is the one this test signs and verifies with.
        let seed: [u8; SEED_BYTES] = digest.of(b"rivet-update test seed, not a key");
        Ed25519Signer::from_seed(&seed)
    }

    /// The boot half a card carries, in `ADR-029` §6's order.
    fn boot_invariant() -> Vec<(&'static str, Vec<u8>)> {
        vec![
            ("kernel8.img", vec![0xa1; 64]),
            ("start4.elf", b"start4".to_vec()),
            ("config.txt", b"arm_64bit=1\ntryboot_a_b=1\n".to_vec()),
            ("tryboot.txt", b"arm_64bit=1\n".to_vec()),
        ]
    }

    struct Built {
        bytes: Vec<u8>,
        system: Vec<u8>,
    }

    /// A real, signed `.rvt` for `product`/`arch`/`platform`, with one system
    /// member, four invariant boot files and `cmdline.txt` for both slots.
    ///
    /// `extra` is appended as a boot-directory member, which is how the
    /// reserved-name test offers an `autoboot.txt`.
    fn build(
        product: &str,
        arch: &str,
        platform: &str,
        version: &str,
        system_bytes: usize,
        extra: Option<(&str, Vec<u8>)>,
    ) -> Built {
        let mut digest = Blake3::new();
        let mut builder = Builder::new(product, version, arch, Some(platform), Kind::update())
            .expect("the header fields fit");
        let mut contents: Vec<Vec<u8>> = Vec::new();

        // A system member whose every byte differs, so a flipped bit anywhere is
        // a real change rather than one the digest could miss.
        let system: Vec<u8> = (0..system_bytes).map(|i| (i % 251) as u8).collect();
        builder
            .add(
                NewMember::stored(
                    "system.img",
                    TargetClass::WholePartition.as_byte(),
                    SlotVariance::Invariant.as_byte(),
                    &system,
                    &mut digest,
                )
                .expect("a well-formed member"),
            )
            .expect("the system member");
        contents.push(system.clone());

        for (name, bytes) in boot_invariant() {
            builder
                .add(
                    NewMember::stored(
                        name,
                        TargetClass::BootDirectory.as_byte(),
                        SlotVariance::Invariant.as_byte(),
                        &bytes,
                        &mut digest,
                    )
                    .expect("a well-formed member"),
                )
                .expect("a boot member");
            contents.push(bytes);
        }
        for (variance, text) in [
            (SlotVariance::SlotA, b"root=PARTLABEL=system-a".to_vec()),
            (SlotVariance::SlotB, b"root=PARTLABEL=system-b".to_vec()),
        ] {
            builder
                .add(
                    NewMember::stored(
                        "cmdline.txt",
                        TargetClass::BootDirectory.as_byte(),
                        variance.as_byte(),
                        &text,
                        &mut digest,
                    )
                    .expect("a well-formed member"),
                )
                .expect("the slot-variant member");
            contents.push(text);
        }
        if let Some((name, bytes)) = extra {
            builder
                .add(
                    NewMember::stored(
                        name,
                        TargetClass::BootDirectory.as_byte(),
                        SlotVariance::Invariant.as_byte(),
                        &bytes,
                        &mut digest,
                    )
                    .expect("a well-formed member"),
                )
                .expect("the extra member");
            contents.push(bytes);
        }

        let borrowed: Vec<&[u8]> = contents.iter().map(Vec::as_slice).collect();
        let mut signer = signer();
        let bytes = builder
            .seal_in_memory(&mut digest, &mut signer, &borrowed)
            .expect("a sealed payload");
        Built { bytes, system }
    }

    fn machine() -> Machine {
        Machine {
            product: "rivet".to_string(),
            platform: "rpi4".to_string(),
            version: "0.0.1".to_string(),
        }
    }

    /// What a Pi 4's image records at `rivet_core::package::arch::PATH`.
    fn records_aarch64(k: &mut FakeKernel) {
        k.put(
            rivet_core::package::arch::PATH,
            rivet_core::package::arch::render("aarch64").expect("a token"),
        );
    }

    /// A card running slot A, so an update targets slot B (`p2`/`p4`).
    fn card(payload: &[u8]) -> FakeKernel {
        let mut k = FakeKernel::new();
        records_aarch64(&mut k);
        k.bytes_mut().insert(
            DT_BOOT_PARTITION.to_string(),
            Script::always(Ok(vec![0, 0, 0, 1])),
        );
        k.files_mut().insert(
            format!("{AUTOBOOT_PARTITION_MOUNT}/autoboot.txt"),
            Script::always(Ok(
                "[all]\ntryboot_a_b=1\nboot_partition=1\n[tryboot]\nboot_partition=1\n".to_string(),
            )),
        );
        k.put(KEY_PATH, signer().public_key().0.to_vec());
        k.put(PAYLOAD_PATH, payload.to_vec());
        // The inactive slot's system partition: 640 MiB declared, and whatever
        // the previous install left in it.
        k.put(&format!("{BOOT_DISK}p4"), vec![0xee; SYSTEM_BYTES]);
        k.declare_size(&format!("{BOOT_DISK}p4"), 640 * 1024 * 1024);
        k
    }

    fn keys() -> Vec<String> {
        vec![KEY_PATH.to_string()]
    }

    fn install(k: &mut FakeKernel, dry_run: bool, read_back: bool) -> Outcome {
        let mut journal = Journal::new("test".to_string(), 1, "update".to_string());
        let corr = journal.next_correlation();
        let mut digest = Blake3::new();
        let verifier = Ed25519Verifier::new();
        let trusted = keys();
        let request = Request {
            payload: PAYLOAD_PATH,
            trusted_keys: &trusted,
            dry_run,
            read_back,
        };
        run(
            k,
            &PiFirmware,
            &mut journal,
            &corr,
            &machine(),
            &request,
            &mut digest,
            &verifier,
        )
    }

    /// Install with **nothing on the command line**, the way a board does.
    fn install_from_the_store(k: &mut FakeKernel) -> Outcome {
        let mut journal = Journal::new("test".to_string(), 1, "update".to_string());
        let corr = journal.next_correlation();
        let mut digest = Blake3::new();
        let verifier = Ed25519Verifier::new();
        let request = Request {
            payload: PAYLOAD_PATH,
            trusted_keys: &[],
            dry_run: true,
            read_back: false,
        };
        run(
            k,
            &PiFirmware,
            &mut journal,
            &corr,
            &machine(),
            &request,
            &mut digest,
            &verifier,
        )
    }

    /// A store carrying one key, entitled to whatever is asked for.
    fn store_with(kinds: &[Kind]) -> Vec<u8> {
        trust_store::TrustStore::new(vec![trust_store::Entry {
            key: signer().public_key(),
            label: "rivet-system".to_string(),
            may_sign: MaySign::these(kinds),
        }])
        .to_bytes()
        .expect("a store of one entry")
    }

    // -----------------------------------------------------------------------
    // The machine's own trust store — ADR-028 5.8.
    // -----------------------------------------------------------------------

    /// **The ordinary path on a real board**: no key is named, and the store
    /// the image carries is the one that answers.
    #[test]
    fn a_card_that_carries_a_trust_store_verifies_with_no_key_on_the_command_line() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        k.put(trust_store::PATH, store_with(&[Kind::update()]));
        assert!(
            matches!(install_from_the_store(&mut k), Outcome::Planned { .. }),
            "a card carrying the signing key refused its own update"
        );
    }

    /// A card built `--no-trust` has no store, and the refusal has to name both
    /// ways out — otherwise the reader knows only that something is missing.
    #[test]
    fn a_card_with_no_trust_store_is_refused_and_names_both_ways_out() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        let Outcome::RefusedMachine(why) = install_from_the_store(&mut k) else {
            panic!("a card with no trust store installed an update");
        };
        assert!(why.contains(trust_store::PATH), "{why}");
        assert!(why.contains("--trusted-key"), "{why}");
        assert!(why.contains("--no-trust"), "{why}");
    }

    /// An empty store is a card that trusts nobody, said in those words rather
    /// than as a parse failure.
    #[test]
    fn an_empty_trust_store_says_it_is_what_a_no_trust_card_looks_like() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        let empty = trust_store::TrustStore::new(Vec::new())
            .to_bytes()
            .expect("an empty store");
        k.put(trust_store::PATH, empty);
        let Outcome::RefusedMachine(why) = install_from_the_store(&mut k) else {
            panic!("an empty trust store authorised an install");
        };
        assert!(why.contains("empty"), "{why}");
        assert!(why.contains("--no-trust"), "{why}");
    }

    /// **A store that cannot be read is not a store that trusts nobody.**
    /// Skipping a corrupt file and carrying on with an empty set would turn a
    /// damaged card into one that refuses everything — which is survivable —
    /// but the same shrug applied anywhere else is how a corrupt file becomes
    /// an install nobody authorised. It is refused, by name.
    #[test]
    fn a_corrupt_trust_store_is_refused_rather_than_treated_as_trusting_nobody() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        let mut damaged = store_with(&[Kind::update()]);
        damaged[0] ^= 0xff;
        k.put(trust_store::PATH, damaged);
        let Outcome::RefusedMachine(why) = install_from_the_store(&mut k) else {
            panic!("a corrupt trust store authorised an install");
        };
        assert!(why.contains("does not parse"), "{why}");
    }

    /// **The store's own mask decides**, and this is the case that proves it:
    /// the key is right, the signature is genuine, and it may not sign an
    /// update. ADR-028 5.5.1's blast radius argument, reached through the
    /// machine's own file rather than through a flag.
    #[test]
    fn a_key_the_store_trusts_only_for_applications_cannot_authorise_an_update() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        k.put(trust_store::PATH, store_with(&[Kind::application()]));
        let outcome = install_from_the_store(&mut k);
        let Outcome::RefusedPayload(why) = outcome else {
            panic!("an application-only key authorised a system update: {outcome:?}");
        };
        // Named rather than merely refused, so this cannot pass for the wrong
        // reason: `UntrustedKey` would mean the store was not read at all, and
        // the two failures are indistinguishable from the variant alone.
        assert!(why.contains("rivet-system"), "{why}");
        assert!(why.contains("update"), "{why}");
        assert!(why.contains("application"), "{why}");
    }

    // -----------------------------------------------------------------------
    // Installing an application — ADR-015 §7.
    // -----------------------------------------------------------------------

    /// A signed application package: a manifest, a program, and one asset.
    fn built_application(name: &str, version: &str) -> Vec<u8> {
        built_application_for(name, version, "aarch64")
    }

    /// [`built_application`], for `arch`.
    fn built_application_for(name: &str, version: &str, arch: &str) -> Vec<u8> {
        built_application_with(name, version, arch, b"a banner\n")
    }

    /// [`built_application_for`], with this asset — a second build of the same
    /// version whose bytes differ.
    fn built_application_with(name: &str, version: &str, arch: &str, banner: &[u8]) -> Vec<u8> {
        use rivet_core::package::application::{
            Manifest, Memory, Network, ProgramKind, Ready, Restart, StateGrant, TargetClass as App,
            VARIANT_NONE,
        };
        let mut digest = Blake3::new();
        let mut builder = Builder::new(name, version, arch, None, Kind::application())
            .expect("the header fields fit");
        let manifest = Manifest::new(ProgramKind::Daemon)
            .with_restart(Restart::OnFailure)
            .with_ready(Ready::Listening)
            .with_network(Network::None)
            .with_state(StateGrant::Private)
            .with_memory(Some(Memory::parse("128M").expect("a memory value")))
            .render()
            .into_bytes();
        // Every byte differs, so a wrong write is a real difference rather than
        // one a comparison could miss.
        let program: Vec<u8> = (0..4096usize).map(|i| (i % 251) as u8).collect();
        let asset = banner.to_vec();

        let mut contents: Vec<Vec<u8>> = Vec::new();
        let parts: Vec<(&str, u8, Vec<u8>)> = vec![
            (MANIFEST_NAME, App::Manifest.as_byte(), manifest),
            // NOT the application's name, so the test proves the program lands
            // at a path derived from the header rather than from the member.
            ("the-crates-binary", App::Program.as_byte(), program),
            ("banner.txt", App::Asset.as_byte(), asset),
        ];
        for (member_name, class, bytes) in parts {
            builder
                .add(
                    NewMember::stored(member_name, class, VARIANT_NONE, &bytes, &mut digest)
                        .expect("a well-formed member"),
                )
                .expect("a member");
            contents.push(bytes);
        }
        let borrowed: Vec<&[u8]> = contents.iter().map(Vec::as_slice).collect();
        let mut signer = signer();
        builder
            .seal_in_memory(&mut digest, &mut signer, &borrowed)
            .expect("a sealed payload")
    }

    /// A machine with a trust store and the payload on it. No slots, no
    /// autoboot: an application install touches none of that, and a fake that
    /// supplied them would hide it if this reached for one.
    fn board_with(payload: &[u8]) -> FakeKernel {
        let mut k = FakeKernel::new();
        records_aarch64(&mut k);
        k.put(
            trust_store::PATH,
            trust_store::TrustStore::new(vec![trust_store::Entry {
                key: signer().public_key(),
                label: "test-apps".to_string(),
                may_sign: MaySign::these(&[Kind::application()]),
            }])
            .to_bytes()
            .expect("a store"),
        );
        k.put(PAYLOAD_PATH, payload.to_vec());
        k
    }

    fn install_app(k: &mut FakeKernel, dry_run: bool) -> Outcome {
        let mut journal = Journal::new("test".to_string(), 1, "update".to_string());
        let corr = journal.next_correlation();
        let mut digest = Blake3::new();
        let verifier = Ed25519Verifier::new();
        let request = Request {
            payload: PAYLOAD_PATH,
            trusted_keys: &[],
            dry_run,
            read_back: false,
        };
        run(
            k,
            &PiFirmware,
            &mut journal,
            &corr,
            &machine(),
            &request,
            &mut digest,
            &verifier,
        )
    }

    /// **The whole install**, checked at the filesystem rather than at the
    /// return value: the directories, the modes, the bytes, and the link.
    #[test]
    fn a_signed_application_becomes_files_under_state_app_and_current_points_at_it() {
        let mut k = board_with(&built_application("photo-index", "0.1.0"));
        let outcome = install_app(&mut k, false);

        let Outcome::InstalledApplication {
            name,
            version,
            files,
        } = &outcome
        else {
            panic!("{outcome:?}");
        };
        assert_eq!(name, "photo-index");
        assert_eq!(version, "0.1.0");
        assert_eq!(
            *files, 5,
            "a manifest, a program, one asset, the signed header and the recorded size (R34)"
        );

        assert_eq!(
            k.mkdirs,
            vec![
                ("/state/app".to_string(), 0o755),
                ("/state/app/photo-index".to_string(), 0o755),
                ("/state/app/photo-index/0.1.0".to_string(), 0o755),
                ("/state/app/photo-index/0.1.0/bin".to_string(), 0o755),
            ]
        );

        // The program is at the path the SUPERVISOR will exec, reached through
        // `current` — which is the property that matters and the one a literal
        // would not check.
        let program = k
            .device_content("/state/app/photo-index/0.1.0/bin/photo-index")
            .expect("the program is written under the application's own name");
        assert_eq!(program.len(), 4096);
        assert!(
            k.device_content("/state/app/photo-index/0.1.0/manifest.toml")
                .is_some()
        );

        // **The retained header is the payload's own first 4096 bytes.**
        // `ADR-015` §4.2's launch verification is only worth anything if this
        // is true: the supervisor re-checks a signature over these bytes, so
        // writing anything else — a re-read, a re-encode, a truncation — would
        // put a file on `/state` that cannot verify and would turn every launch
        // into a refusal. Compared against the payload rather than against a
        // constant, so the format changing does not quietly make this vacuous.
        let payload = k
            .device_content(PAYLOAD_PATH)
            .expect("the fake still holds the payload it installed from");
        let header = k
            .device_content("/state/app/photo-index/0.1.0/package-header")
            .expect("ADR-015 §4.2 needs something signed beside the install");
        assert_eq!(header.len(), HEADER_BYTES);
        assert_eq!(
            header,
            &payload[..HEADER_BYTES],
            "the retained header is not the bytes the signature was checked over"
        );
        assert!(
            k.device_content("/state/app/photo-index/0.1.0/banner.txt")
                .is_some()
        );

        // **R34, `.rulings/` R-163: the recorded size is the sum of every
        // file this install actually wrote into the version directory** —
        // the program, the manifest, the asset and the retained header —
        // and NOT the size record's own bytes (§`Step::Size`'s own doc says
        // why: it would be a directory whose recorded total describes a
        // directory holding one more file than the number that was summed).
        let program_len = program.len() as u64;
        let manifest_len = k
            .device_content("/state/app/photo-index/0.1.0/manifest.toml")
            .expect("the manifest was written")
            .len() as u64;
        let banner_len = k
            .device_content("/state/app/photo-index/0.1.0/banner.txt")
            .expect("the asset was written")
            .len() as u64;
        let header_len = header.len() as u64;
        let recorded = k
            .device_content("/state/app/photo-index/0.1.0/install-size")
            .expect("R34 records the size beside the header");
        assert_eq!(
            String::from_utf8_lossy(recorded).trim(),
            (program_len + manifest_len + banner_len + header_len).to_string(),
            "the recorded size must equal the sum of the files this install actually wrote"
        );
        assert!(
            k.chmods
                .contains(&("/state/app/photo-index/0.1.0/install-size".to_string(), 0o444)),
            "the recorded size is read-only like the rest of the version tree: {:?}",
            k.chmods
        );

        // ...and nothing under the member's own name.
        assert!(
            !k.paths().iter().any(|p| p.contains("the-crates-binary")),
            "{:?}",
            k.paths()
        );

        assert!(
            k.chmods.contains(&(
                "/state/app/photo-index/0.1.0/bin/photo-index".to_string(),
                0o555
            )),
            "{:?}",
            k.chmods
        );

        assert_eq!(
            k.links
                .get("/state/app/photo-index/current")
                .map(String::as_str),
            Some("0.1.0"),
            "current does not point at the version just installed: {:?}",
            k.links
        );
    }

    /// **The link is the last thing that happens.** Nothing is written after the
    /// rename, so a machine that dies mid-install has an unreferenced version
    /// directory and an unchanged `current` — which is `ADR-015` §7's whole
    /// argument about a Pi whose power is pulled.
    #[test]
    fn nothing_is_written_after_the_link_is_swapped() {
        let mut k = board_with(&built_application("photo-index", "0.1.0"));
        assert!(matches!(
            install_app(&mut k, false),
            Outcome::InstalledApplication { .. }
        ));
        let (_, link_to) = k
            .renames
            .iter()
            .find(|(from, _)| from.ends_with(".current.new"))
            .expect("the link was renamed");
        assert_eq!(link_to, "/state/app/photo-index/current");
        assert_eq!(
            k.renames.len(),
            1,
            "an application install renames exactly once: {:?}",
            k.renames
        );
        // **The rename is the LAST filesystem operation.** Asserted over the
        // fake's ordered log rather than over the per-kind vectors, because
        // those cannot say what came before what — this test passed with the
        // steps run in reverse until the log existed.
        assert_eq!(
            k.ops.last().map(String::as_str),
            Some("rename /state/app/photo-index/.current.new -> /state/app/photo-index/current"),
            "{:?}",
            k.ops
        );
        // ...and the first is the outermost directory — `/state/app` itself
        // since 2026-08-25 — so the plan's order
        // survived into the machine.
        assert_eq!(
            k.ops.first().map(String::as_str),
            Some("mkdir /state/app"),
            "{:?}",
            k.ops
        );
        // The staging link is gone from `links` because the rename moved it.
        assert!(!k.links.contains_key("/state/app/photo-index/.current.new"));
    }

    /// A dry run reaches the same verdict and writes nothing at all — no
    /// directory, no file, no link. `ADR-006` §6.2.
    #[test]
    fn a_dry_run_of_an_application_writes_nothing_and_still_says_what_it_would_do() {
        let mut k = board_with(&built_application("photo-index", "0.1.0"));
        let outcome = install_app(&mut k, true);
        let Outcome::PlannedApplication {
            name,
            version,
            steps,
        } = &outcome
        else {
            panic!("{outcome:?}");
        };
        assert_eq!(name, "photo-index");
        assert_eq!(version, "0.1.0");
        assert_eq!(
            *steps, 10,
            "4 directories, 3 members, the signed header, the recorded size (R34), 1 activation"
        );
        assert_eq!(outcome.wrote_nothing(), Some(true));
        assert!(k.mkdirs.is_empty(), "{:?}", k.mkdirs);
        assert!(k.synced_writes.is_empty(), "{:?}", k.synced_writes);
        assert!(k.links.is_empty(), "{:?}", k.links);
        assert!(k.renames.is_empty(), "{:?}", k.renames);
    }

    /// **A tampered member is refused and NOTHING reaches the filesystem** —
    /// not a directory, not the good members that came before the bad one.
    /// `ADR-029` §5 step 8 verifies every member before step 7 writes any.
    #[test]
    fn a_tampered_application_writes_nothing_at_all() {
        let mut payload = built_application("photo-index", "0.1.0");
        // Flip a byte deep inside the program member — past the header, so the
        // signature still covers a header that parses and the failure is the
        // member digest rather than the signature.
        let at = payload.len() - 100;
        payload[at] ^= 0xff;
        let mut k = board_with(&payload);
        let outcome = install_app(&mut k, false);
        assert!(matches!(outcome, Outcome::RefusedPayload(_)), "{outcome:?}");
        assert_eq!(outcome.wrote_nothing(), Some(true));
        assert!(k.mkdirs.is_empty(), "{:?}", k.mkdirs);
        assert!(k.synced_writes.is_empty(), "{:?}", k.synced_writes);
        assert!(k.links.is_empty(), "{:?}", k.links);
    }

    /// **The installer keeps its own check, and this is why.** Since
    /// 2026-09-12 the artifact store's door refuses an artifact that does not
    /// verify (`ADR-052` §7), so nothing the bulk endpoint stored can be
    /// this. But the store is not the only way bytes reach `/state` — anything
    /// running as root can write there — and a store's verdict is true of the
    /// trust store it was reached against, which an image update replaces.
    ///
    /// So: an application sitting at an object path in the store, signed by a
    /// key this machine's trust store does NOT hold, handed straight to the
    /// installer. It is refused from the machine's own store, through the same
    /// `rivet_core::package::verify` the door calls, and nothing is written.
    #[test]
    fn an_untrusted_artifact_that_reached_the_store_by_another_route_is_still_refused() {
        let object = format!(
            "{}/objects/{}/photo-index-0.1.0-aarch64-application.rvt",
            rivet_core::state::ARTIFACTS,
            "ab".repeat(32)
        );
        let mut k = board_with(&built_application("photo-index", "0.1.0"));
        k.put(&object, built_application("photo-index", "0.1.0"));
        // The machine trusts somebody else.
        let mut digest = Blake3::new();
        let stranger = Ed25519Signer::from_seed(&digest.of(b"a key this test did not sign with"));
        k.put(
            trust_store::PATH,
            trust_store::TrustStore::new(vec![trust_store::Entry {
                key: stranger.public_key(),
                label: "someone-else".to_string(),
                may_sign: MaySign::these(&[Kind::application()]),
            }])
            .to_bytes()
            .expect("a store"),
        );

        let mut journal = Journal::new("test".to_string(), 1, "update".to_string());
        let corr = journal.next_correlation();
        let verifier = Ed25519Verifier::new();
        let request = Request {
            payload: &object,
            trusted_keys: &[],
            dry_run: false,
            read_back: false,
        };
        let outcome = run(
            &mut k,
            &PiFirmware,
            &mut journal,
            &corr,
            &machine(),
            &request,
            &mut Blake3::new(),
            &verifier,
        );
        let Outcome::RefusedPayload(why) = &outcome else {
            panic!("an artifact signed by an untrusted key was not refused: {outcome:?}");
        };
        assert!(why.contains("not in this machine's trust set"), "{why}");
        assert_eq!(outcome.wrote_nothing(), Some(true));
        assert!(k.mkdirs.is_empty(), "{:?}", k.mkdirs);
        assert!(k.synced_writes.is_empty(), "{:?}", k.synced_writes);
        assert!(k.links.is_empty(), "{:?}", k.links);
    }

    /// **A crash between staging the link and renaming it must not brick the
    /// application for ever.** `symlink(2)` answers `EEXIST`, so the staging
    /// name is removed first — and this is the test that says the recovery
    /// works rather than that the code looks like it should.
    #[test]
    fn a_staging_link_left_by_an_earlier_crash_does_not_block_the_next_install() {
        let mut k = board_with(&built_application("photo-index", "0.1.0"));
        k.links.insert(
            "/state/app/photo-index/.current.new".to_string(),
            "0.0.9-half-installed".to_string(),
        );
        let outcome = install_app(&mut k, false);
        assert!(
            matches!(outcome, Outcome::InstalledApplication { .. }),
            "{outcome:?}"
        );
        assert_eq!(
            k.links
                .get("/state/app/photo-index/current")
                .map(String::as_str),
            Some("0.1.0")
        );
        assert!(
            k.unlinks
                .contains(&"/state/app/photo-index/.current.new".to_string()),
            "{:?}",
            k.unlinks
        );
    }

    /// A second version installs beside the first and `current` moves. The old
    /// version is left where it is — which is what makes going back a symlink
    /// swap, and is also why nothing here reclaims disk.
    #[test]
    fn a_second_version_lands_beside_the_first_and_the_link_moves() {
        let mut k = board_with(&built_application("photo-index", "0.1.0"));
        assert!(matches!(
            install_app(&mut k, false),
            Outcome::InstalledApplication { .. }
        ));
        k.put(PAYLOAD_PATH, built_application("photo-index", "0.2.0"));
        assert!(matches!(
            install_app(&mut k, false),
            Outcome::InstalledApplication { .. }
        ));

        assert_eq!(
            k.links
                .get("/state/app/photo-index/current")
                .map(String::as_str),
            Some("0.2.0")
        );
        assert!(
            k.device_content("/state/app/photo-index/0.1.0/bin/photo-index")
                .is_some(),
            "the first version was removed, so there is nothing to go back to"
        );
        assert!(
            k.device_content("/state/app/photo-index/0.2.0/bin/photo-index")
                .is_some()
        );
    }

    // -- ADR-015 §7.2: nothing unpacks over a directory that is there --------

    /// **The defect, as a test.** Installing a payload whose version matches one
    /// already on the machine used to write its files straight into the live
    /// directory — the one `current` points at, holding the program the
    /// supervisor execs and the signed header `ADR-015` §4.2 re-verifies at every
    /// launch. It is refused, and the assertion that matters is that the second
    /// attempt touched **nothing**.
    ///
    /// **A second BUILD of the version, since 2026-09-12**: the same bytes again
    /// are what `current` already runs, and are refused as that
    /// (`the_application_this_machine_runs_is_refused_as_already_running`).
    #[test]
    fn a_second_install_of_the_same_version_is_refused_and_writes_nothing() {
        let mut k = board_with(&built_application("photo-index", "0.1.0"));
        assert!(matches!(
            install_app(&mut k, false),
            Outcome::InstalledApplication { .. }
        ));

        let after_first: Vec<(String, Vec<u8>)> = k.synced_writes.clone();
        let links_after_first = k.links.clone();
        let ops_after_first = k.ops.len();

        k.put(
            PAYLOAD_PATH,
            built_application_with("photo-index", "0.1.0", "aarch64", b"another banner\n"),
        );
        let outcome = install_app(&mut k, false);
        let Outcome::RefusedPayload(why) = outcome else {
            panic!("a second install of one version was not refused: {outcome:?}");
        };
        assert!(why.contains("/state/app/photo-index/0.1.0"), "{why}");
        assert!(why.contains("ADR-015 §7.2"), "{why}");

        assert_eq!(
            k.synced_writes, after_first,
            "the refused install wrote files"
        );
        assert_eq!(k.links, links_after_first, "the refused install moved a link");
        assert_eq!(
            k.ops.len(),
            ops_after_first,
            "the refused install performed filesystem operations: {:?}",
            &k.ops[ops_after_first..]
        );
    }

    // -- PLAN-Feature-Set-2026-09-12 §3: what this machine is already running ----------------------

    /// **Re-sending the application `current` selects is refused as what it
    /// is**: this machine already runs these bytes, nothing is written, and the
    /// refusal carries the words a host tool stops on.
    #[test]
    fn the_application_this_machine_runs_is_refused_as_already_running() {
        let mut k = board_with(&built_application("photo-index", "0.1.0"));
        assert!(matches!(
            install_app(&mut k, false),
            Outcome::InstalledApplication { .. }
        ));
        let ops_after_first = k.ops.len();

        let outcome = install_app(&mut k, false);
        let Outcome::RefusedPayload(why) = &outcome else {
            panic!("a re-send of the running application was not refused: {outcome:?}");
        };
        assert!(why.contains(rivet_core::deploy::ALREADY_RUNNING), "{why}");
        assert!(why.contains("photo-index"), "{why}");
        assert_eq!(k.ops.len(), ops_after_first, "{:?}", &k.ops[ops_after_first..]);
    }

    /// **A version that is on the machine and not `current` is not "running".**
    /// It is refused, as before, because its directory is occupied — and the
    /// refusal must not claim this machine runs it.
    #[test]
    fn a_version_on_the_machine_that_current_does_not_select_is_not_called_running() {
        let v1 = built_application("photo-index", "0.1.0");
        let mut k = board_with(&v1);
        assert!(matches!(install_app(&mut k, false), Outcome::InstalledApplication { .. }));
        k.put(PAYLOAD_PATH, built_application("photo-index", "0.2.0"));
        assert!(matches!(install_app(&mut k, false), Outcome::InstalledApplication { .. }));

        k.put(PAYLOAD_PATH, v1);
        let Outcome::RefusedPayload(why) = install_app(&mut k, false) else {
            panic!("an occupied version directory was installed into");
        };
        assert!(!why.contains(rivet_core::deploy::ALREADY_RUNNING), "{why}");
        assert!(why.contains("/state/app/photo-index/0.1.0"), "{why}");
    }

    /// A version directory left behind by **an earlier boot** is the same
    /// refusal, and this is the case a single-process test would otherwise miss:
    /// nothing in this run created the directory.
    #[test]
    fn a_version_directory_from_an_earlier_boot_is_refused_too() {
        let mut k = board_with(&built_application("photo-index", "0.1.0"));
        k.already_there("/state/app/photo-index/0.1.0");
        let outcome = install_app(&mut k, false);
        let Outcome::RefusedPayload(why) = outcome else {
            panic!("an occupied version directory was installed into: {outcome:?}");
        };
        assert!(why.contains("NOTHING WAS WRITTEN"), "{why}");
        assert!(
            k.synced_writes.is_empty() && k.mkdirs.is_empty(),
            "the refusal came after something had already been done: {:?} {:?}",
            k.synced_writes,
            k.mkdirs
        );
    }

    /// **A dry run must refuse it too.** A `--dry-run` that answers *this would
    /// work* about an install that will be refused is worse than no dry run —
    /// it is the answer an operator plans a deploy window around.
    #[test]
    fn a_dry_run_over_an_occupied_version_directory_refuses_rather_than_planning() {
        let mut k = board_with(&built_application("photo-index", "0.1.0"));
        k.already_there("/state/app/photo-index/0.1.0");
        let outcome = install_app(&mut k, true);
        assert!(
            matches!(outcome, Outcome::RefusedPayload(_)),
            "a dry run planned an install that would be refused: {outcome:?}"
        );
    }

    /// The refusal is in the **journal**, not only in the return value: the
    /// board's own record is what `rivet get install` reads and what an operator
    /// who was not watching stdout has.
    #[test]
    fn the_refusal_reaches_the_journal_at_warn() {
        let mut k = board_with(&built_application("photo-index", "0.1.0"));
        k.already_there("/state/app/photo-index/0.1.0");
        let _ = install_app(&mut k, false);
        assert!(
            k.records
                .iter()
                .any(|r| r.msg.contains("ADR-015 §7.2") && r.sev == Severity::Warn),
            "the refusal is not in the journal: {:?}",
            k.messages()
        );
    }

    // -----------------------------------------------------------------------
    // Installing a library — Shared Rivets, 2026-08-31.
    // -----------------------------------------------------------------------

    fn library_manifest_bytes() -> Vec<u8> {
        rivet_core::package::library::Manifest::new(vec![
            "libgreet.so.1".to_string(),
            "libgreet.so.2".to_string(),
        ])
        .expect("a well-formed soname list")
        .render()
        .into_bytes()
    }

    /// Every byte differs between the two objects an `offset` apart, so a
    /// wrong write — or a wrong retry — is a real difference rather than one
    /// a comparison could miss.
    fn library_object_bytes(offset: usize) -> Vec<u8> {
        (0..2048usize).map(|i| ((i + offset) % 251) as u8).collect()
    }

    /// A signed library package: a manifest and two objects, so a fault
    /// injected on the second object's write lands strictly between them.
    /// No asset — the reserved-name refusal has its own coverage in
    /// `rivet-core`.
    fn built_library(name: &str, version: &str) -> Vec<u8> {
        use rivet_core::package::application::VARIANT_NONE;
        use rivet_core::package::library::{MANIFEST_NAME as LIB_MANIFEST_NAME, TargetClass as Lib};

        let mut digest = Blake3::new();
        let mut builder = Builder::new(name, version, "aarch64", None, Kind::library())
            .expect("the header fields fit");

        let parts: Vec<(&str, u8, Vec<u8>)> = vec![
            (LIB_MANIFEST_NAME, Lib::Manifest.as_byte(), library_manifest_bytes()),
            ("libgreet.so.1", Lib::Object.as_byte(), library_object_bytes(0)),
            ("libgreet.so.2", Lib::Object.as_byte(), library_object_bytes(7)),
        ];
        let mut contents: Vec<Vec<u8>> = Vec::new();
        for (member_name, class, bytes) in &parts {
            builder
                .add(
                    NewMember::stored(member_name, *class, VARIANT_NONE, bytes, &mut digest)
                        .expect("a well-formed member"),
                )
                .expect("a member");
            contents.push(bytes.clone());
        }
        let borrowed: Vec<&[u8]> = contents.iter().map(Vec::as_slice).collect();
        let mut signer = signer();
        builder
            .seal_in_memory(&mut digest, &mut signer, &borrowed)
            .expect("a sealed payload")
    }

    /// A machine with a trust store permitting a library, and the payload on
    /// it. No slots, no autoboot: a library install touches none of that.
    fn board_with_library(payload: &[u8]) -> FakeKernel {
        let mut k = FakeKernel::new();
        records_aarch64(&mut k);
        k.put(
            trust_store::PATH,
            trust_store::TrustStore::new(vec![trust_store::Entry {
                key: signer().public_key(),
                label: "test-libs".to_string(),
                may_sign: MaySign::these(&[Kind::library()]),
            }])
            .to_bytes()
            .expect("a store"),
        );
        k.put(PAYLOAD_PATH, payload.to_vec());
        k
    }

    fn install_lib(k: &mut FakeKernel, dry_run: bool) -> Outcome {
        let mut journal = Journal::new("test".to_string(), 1, "update".to_string());
        let corr = journal.next_correlation();
        let mut digest = Blake3::new();
        let verifier = Ed25519Verifier::new();
        let request = Request {
            payload: PAYLOAD_PATH,
            trusted_keys: &[],
            dry_run,
            read_back: false,
        };
        run(
            k,
            &PiFirmware,
            &mut journal,
            &corr,
            &machine(),
            &request,
            &mut digest,
            &verifier,
        )
    }

    /// Where `reference` installs to, using the same public path functions
    /// the installer itself does — never a hand-formatted string, so a test
    /// cannot silently drift from what the code actually writes.
    fn library_paths(reference: &str) -> (String, String) {
        let parsed = rivet_core::package::library::Reference::parse(reference)
            .expect("a reference this crate just produced parses");
        let volume = rivet_core::service::BUILT_IN_VOLUME;
        let dir = rivet_core::package::install::library_dir(volume, &parsed);
        let header = format!("{dir}/{}", rivet_core::package::install::HEADER_NAME);
        (dir, header)
    }

    /// A script that fails the write at 1-based position `at` among a
    /// kernel's next `write_file_synced` calls, and succeeds at every other
    /// position.
    fn fail_write_at(at: usize) -> Script<Result<(), Errno>> {
        let mut steps = vec![Ok(()); at - 1];
        steps.push(Err(Errno::EIO));
        Script::sequence(steps, Ok(()))
    }

    /// **The core of the regression.** A machine where only the DIRECTORY
    /// survived — exactly what an install interrupted right after
    /// `Step::Directory`, before anything else, leaves behind — must not
    /// read as already installed, and installing into it must finish the
    /// job rather than call it done.
    #[test]
    fn a_library_directory_with_no_header_is_not_already_installed() {
        let payload = built_library("libgreet", "1.0.0");

        // Learn where this payload installs, on its own throwaway machine —
        // the reference embeds a digest of the signed header, so nothing
        // here should hand-compute it.
        let mut throwaway = board_with_library(&payload);
        let Outcome::InstalledLibrary { reference, .. } = install_lib(&mut throwaway, false)
        else {
            panic!("could not learn where this payload installs");
        };
        let (dir, header) = library_paths(&reference);

        let mut k = board_with_library(&payload);
        k.already_there(&dir);
        let outcome = install_lib(&mut k, false);
        let Outcome::InstalledLibrary { reference: got, .. } = outcome else {
            panic!("a directory with no header was treated as already installed: {outcome:?}");
        };
        assert_eq!(got, reference);
        assert!(
            k.device_content(&header).is_some(),
            "the header is missing after installing into a bare directory"
        );
    }

    /// **The shared shape of the next three tests**: a write fails at
    /// position `at` among this payload's four writes — the manifest, two
    /// objects and the header, in that order — the attempt is refused, and a
    /// retry on the same machine completes with every file's bytes and the
    /// header present. Never `LibraryAlreadyInstalled`: the header was not
    /// there for the retry to have found.
    fn a_failed_write_recovers_on_retry(at: usize) {
        let payload = built_library("libgreet", "1.0.0");
        let mut k = board_with_library(&payload);
        k.write_results = fail_write_at(at);

        let failed = install_lib(&mut k, false);
        assert!(
            matches!(failed, Outcome::RefusedMachine(_)),
            "write {at} of 4 did not fail the install: {failed:?}"
        );

        k.write_results = Script::always(Ok(()));
        let retried = install_lib(&mut k, false);
        let Outcome::InstalledLibrary { reference, files } = &retried else {
            panic!("the retry after write {at} failed did not complete: {retried:?}");
        };
        assert_eq!(*files, 4, "a manifest, two objects and the header");

        let (dir, header) = library_paths(reference);
        // Objects land under the library's OWN object directory — what a
        // loader is pointed at — never directly under `dir`, which holds
        // only the manifest and the retained header beside it.
        let object_dir = format!("{dir}/{}", rivet_core::package::library::OBJECT_DIR);
        let header_bytes = k
            .device_content(&header)
            .expect("the header is missing after the retry");
        assert_eq!(header_bytes, &payload[..HEADER_BYTES]);
        assert_eq!(
            k.device_content(&format!("{object_dir}/libgreet.so.1")),
            Some(library_object_bytes(0).as_slice())
        );
        assert_eq!(
            k.device_content(&format!("{object_dir}/libgreet.so.2")),
            Some(library_object_bytes(7).as_slice())
        );
        assert_eq!(
            k.device_content(&format!("{dir}/manifest.toml")),
            Some(library_manifest_bytes().as_slice())
        );
    }

    #[test]
    fn a_failure_while_writing_the_manifest_leaves_a_retry_able_to_complete() {
        a_failed_write_recovers_on_retry(1);
    }

    #[test]
    fn a_failure_between_two_objects_leaves_a_retry_able_to_complete() {
        a_failed_write_recovers_on_retry(3);
    }

    #[test]
    fn a_failure_while_writing_the_header_leaves_a_retry_able_to_complete() {
        a_failed_write_recovers_on_retry(4);
    }

    /// A library that finished installing, installed again, is the
    /// documented no-op: reported `LibraryAlreadyInstalled`, and nothing
    /// reaches the filesystem a second time.
    #[test]
    fn a_complete_library_retried_is_reported_already_installed_and_writes_nothing() {
        let payload = built_library("libgreet", "1.0.0");
        let mut k = board_with_library(&payload);
        assert!(matches!(
            install_lib(&mut k, false),
            Outcome::InstalledLibrary { .. }
        ));

        let writes_after_first = k.synced_writes.len();
        let ops_after_first = k.ops.len();

        let outcome = install_lib(&mut k, false);
        assert!(
            matches!(outcome, Outcome::LibraryAlreadyInstalled { .. }),
            "{outcome:?}"
        );
        assert_eq!(
            k.synced_writes.len(),
            writes_after_first,
            "a second install of a complete library wrote something"
        );
        assert_eq!(
            k.ops.len(),
            ops_after_first,
            "a second install of a complete library performed a filesystem operation: {:?}",
            &k.ops[ops_after_first..]
        );
    }

    // -----------------------------------------------------------------------
    // `apk` — `ADR-061` §3: it verifies and is refused by name, never
    // installed.
    // -----------------------------------------------------------------------

    fn built_apk(name: &str, version: &str) -> Vec<u8> {
        use rivet_core::package::apk::{APK_NAME, METADATA_NAME};
        let metadata = "{\"application_id\":\"com.example.gantry\",\"version_code\":1,\
             \"version_name\":\"0.1.0\",\"label\":\"Gantry\",\"min_sdk\":21,\
             \"target_sdk\":34,\"signer_sha256\":\""
            .to_string()
            + &"ab".repeat(32)
            + "\",\"development\":true}";
        let mut digest = Blake3::new();
        let mut builder = Builder::new(name, version, "any", Some("android"), Kind::apk())
            .expect("the header fields fit");
        let parts: Vec<(&str, Vec<u8>)> = vec![
            (APK_NAME, b"pretend-this-is-a-zip".to_vec()),
            (METADATA_NAME, metadata.into_bytes()),
        ];
        let mut contents: Vec<Vec<u8>> = Vec::new();
        for (member_name, bytes) in &parts {
            builder
                .add(
                    NewMember::stored(member_name, 0, 0, bytes, &mut digest)
                        .expect("a well-formed member"),
                )
                .expect("a member");
            contents.push(bytes.clone());
        }
        let borrowed: Vec<&[u8]> = contents.iter().map(Vec::as_slice).collect();
        let mut signer = signer();
        builder
            .seal_in_memory(&mut digest, &mut signer, &borrowed)
            .expect("a sealed payload")
    }

    /// **The application key's mask, since `apk` joined it** — `ADR-028`
    /// §5.5.2, `.rulings/` R-222. A machine whose trust store carries the
    /// same key an application install would use is enough to reach the
    /// installer's kind branch; what this test proves is that branch, not the
    /// entitlement check `rivetos-core`'s own suite already covers.
    fn board_with_apk(payload: &[u8]) -> FakeKernel {
        let mut k = FakeKernel::new();
        records_aarch64(&mut k);
        k.put(
            trust_store::PATH,
            trust_store::TrustStore::new(vec![trust_store::Entry {
                key: signer().public_key(),
                label: "test-apps".to_string(),
                may_sign: MaySign::these(&[Kind::application(), Kind::library(), Kind::apk()]),
            }])
            .to_bytes()
            .expect("a store"),
        );
        k.put(PAYLOAD_PATH, payload.to_vec());
        k
    }

    fn install_apk(k: &mut FakeKernel) -> Outcome {
        let mut journal = Journal::new("test".to_string(), 1, "update".to_string());
        let corr = journal.next_correlation();
        let mut digest = Blake3::new();
        let verifier = Ed25519Verifier::new();
        let request = Request {
            payload: PAYLOAD_PATH,
            trusted_keys: &[],
            dry_run: false,
            read_back: false,
        };
        run(
            k,
            &PiFirmware,
            &mut journal,
            &corr,
            &machine(),
            &request,
            &mut digest,
            &verifier,
        )
    }

    /// **The whole point of `ADR-061` §3**: a well-formed, trusted `apk`
    /// verifies — the header, the signature and the entitlement all check
    /// out — and is refused anyway, by name, before anything about a slot is
    /// touched.
    #[test]
    fn an_apk_verifies_and_is_refused_by_name_never_installed() {
        let payload = built_apk("gantry", "0.1.17");
        let mut k = board_with_apk(&payload);
        let outcome = install_apk(&mut k);
        let Outcome::RefusedPayload(why) = &outcome else {
            panic!("an apk was not refused as a payload: {outcome:?}");
        };
        assert!(why.contains("apk"), "{why}");
        assert!(why.contains("never installed"), "{why}");
        assert!(k.synced_writes.is_empty(), "an apk install wrote something");
    }

    /// An application declaring `closure` as the libraries it needs, built
    /// otherwise exactly as [`built_application`] is — one manifest, one
    /// program, no asset.
    fn built_application_needing(
        name: &str,
        version: &str,
        closure: Vec<rivet_core::package::library::Reference>,
    ) -> Vec<u8> {
        use rivet_core::package::application::{
            Manifest, Memory, Network, ProgramKind, Ready, Restart, StateGrant, TargetClass as App,
            VARIANT_NONE,
        };
        let mut digest = Blake3::new();
        let mut builder = Builder::new(name, version, "aarch64", None, Kind::application())
            .expect("the header fields fit");
        let manifest = Manifest::new(ProgramKind::Daemon)
            .with_restart(Restart::OnFailure)
            .with_ready(Ready::Listening)
            .with_network(Network::None)
            .with_state(StateGrant::Private)
            .with_memory(Some(Memory::parse("128M").expect("a memory value")))
            .with_library(closure)
            .expect("a closure within MAX_CLOSURE")
            .render()
            .into_bytes();
        let program: Vec<u8> = (0..4096usize).map(|i| (i % 251) as u8).collect();

        let mut contents: Vec<Vec<u8>> = Vec::new();
        let parts: Vec<(&str, u8, Vec<u8>)> = vec![
            (MANIFEST_NAME, App::Manifest.as_byte(), manifest),
            ("the-crates-binary", App::Program.as_byte(), program),
        ];
        for (member_name, class, bytes) in parts {
            builder
                .add(
                    NewMember::stored(member_name, class, VARIANT_NONE, &bytes, &mut digest)
                        .expect("a well-formed member"),
                )
                .expect("a member");
            contents.push(bytes);
        }
        let borrowed: Vec<&[u8]> = contents.iter().map(Vec::as_slice).collect();
        let mut signer = signer();
        builder
            .seal_in_memory(&mut digest, &mut signer, &borrowed)
            .expect("a sealed payload")
    }

    /// **The other half of the regression**: the damage an interrupted
    /// library install does to a dependent application's closure check,
    /// never just to the library's own retry.
    #[test]
    fn an_application_needing_a_partially_installed_library_is_refused_as_unsatisfied() {
        let lib_payload = built_library("libgreet", "1.0.0");
        let mut throwaway = board_with_library(&lib_payload);
        let Outcome::InstalledLibrary { reference, .. } = install_lib(&mut throwaway, false)
        else {
            panic!("could not learn where this library installs");
        };
        let (dir, _) = library_paths(&reference);
        let parsed = rivet_core::package::library::Reference::parse(&reference)
            .expect("a reference this crate just produced parses");

        let app_payload = built_application_needing("photo-index", "0.1.0", vec![parsed]);
        let mut k = board_with(&app_payload);
        // The library's directory exists and its header does not — an
        // interrupted install, never a completed one.
        k.already_there(&dir);

        let outcome = install_app(&mut k, false);
        let Outcome::RefusedPayload(why) = &outcome else {
            panic!("an application needing an incomplete library was not refused: {outcome:?}");
        };
        assert!(why.contains(&reference), "{why}");
        assert!(k.mkdirs.is_empty(), "{:?}", k.mkdirs);
        assert!(k.synced_writes.is_empty(), "{:?}", k.synced_writes);
    }

    // -----------------------------------------------------------------------
    // The happy path.
    // -----------------------------------------------------------------------

    #[test]
    fn a_signed_payload_reaches_the_inactive_slot_and_the_next_boot_is_a_trial() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        let outcome = install(&mut k, false, true);

        let Outcome::Installed {
            slot,
            armed,
            system_bytes,
            boot_written,
            boot_skipped,
        } = outcome
        else {
            panic!("{outcome:?}");
        };
        // Running p1, so the update writes p2 / p4.
        assert_eq!(slot, SlotId(2));
        assert_eq!(armed, ArmedReboot::OneShot);
        assert_eq!(system_bytes, SYSTEM_BYTES as u64);
        assert_eq!(boot_written, 5, "four invariant files and one cmdline.txt");
        assert_eq!(boot_skipped, 0);

        // The slot holds exactly the member's bytes.
        assert_eq!(
            k.device_content(&format!("{BOOT_DISK}p4")),
            Some(built.system.as_slice())
        );
        // Slot B's cmdline.txt, and not slot A's.
        let written = k
            .synced_writes
            .iter()
            .find(|(p, _)| p.ends_with("cmdline.txt.new"))
            .map(|(_, c)| c.clone())
            .expect("cmdline.txt was not written");
        assert_eq!(written, b"root=PARTLABEL=system-b".to_vec());
        // And the arming moved the [tryboot] line to partition 2.
        assert!(
            k.synced_writes
                .iter()
                .any(|(_, c)| String::from_utf8_lossy(c).contains("[tryboot]\nboot_partition=2")),
            "{:?}",
            k.synced_writes.iter().map(|(p, _)| p).collect::<Vec<_>>()
        );
    }

    /// **`ADR-029` §9's second bound on F5**, asserted over what the machine
    /// actually saw rather than over the plan: the system partition is written
    /// before the boot half is opened for writing at all.
    #[test]
    fn the_system_member_is_written_before_the_boot_half_is_mounted_writable() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        assert!(matches!(
            install(&mut k, false, true),
            Outcome::Installed { .. }
        ));

        assert_eq!(k.device_writes.len(), 1, "{:?}", k.device_writes);
        // Every writable mount happens after the one device write. There is no
        // clock here, so the ordering is asserted the only way it can be: the
        // first writable mount is of the boot half, and the device write is the
        // only one there is.
        let first_writable = k
            .mount_calls
            .iter()
            .position(|c| c.writable)
            .expect("nothing was ever mounted writable");
        let read_only_before = k.mount_calls[..first_writable].iter().all(|c| !c.writable);
        assert!(read_only_before);
        assert_eq!(
            k.mount_calls[first_writable].device,
            format!("{BOOT_DISK}p2"),
            "the first writable mount is not the boot half of the target slot"
        );
    }

    /// `ADR-029` §9.1: install and arm are one operation, **and the reboot is
    /// separate.** Nothing on any port here can restart the machine, and this
    /// asserts the absence rather than trusting it.
    #[test]
    fn an_install_arms_and_does_not_reboot() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        let outcome = install(&mut k, false, true);
        assert!(matches!(
            outcome,
            Outcome::Installed {
                armed: ArmedReboot::OneShot,
                ..
            }
        ));
        assert!(
            k.messages().iter().any(|m| m.contains("trial")),
            "nothing said the next boot is a trial: {:?}",
            k.messages()
        );
    }

    // -----------------------------------------------------------------------
    // The refusals. Each asserts the slot is untouched.
    // -----------------------------------------------------------------------

    /// The slot's bytes before an install, so a refusal can be shown to have
    /// changed nothing.
    fn slot_before(k: &FakeKernel) -> Vec<u8> {
        k.device_content(&format!("{BOOT_DISK}p4"))
            .expect("the slot")
            .to_vec()
    }

    /// **The one that matters more than the happy path.** `ADR-029` §5 step 8:
    /// *"refuse on mismatch, having written nothing further"* — and F3: the
    /// slot is left as it is, not wiped.
    #[test]
    fn a_flipped_bit_in_a_member_is_refused_and_the_slot_is_untouched() {
        let mut built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let at = HEADER_BYTES + SYSTEM_BYTES / 2;
        built.bytes[at] ^= 0x01;
        let mut k = card(&built.bytes);
        let before = slot_before(&k);

        let outcome = install(&mut k, false, true);
        assert!(matches!(outcome, Outcome::RefusedPayload(_)), "{outcome:?}");
        assert_eq!(outcome.wrote_nothing(), Some(true));
        assert!(k.device_writes.is_empty(), "{:?}", k.device_writes);
        assert!(k.synced_writes.is_empty(), "{:?}", k.synced_writes);
        assert_eq!(slot_before(&k), before, "the slot changed");
    }

    /// **`ADR-029` §9.2 step 6 says *every* member, and this is the half that
    /// is easy to get wrong.**
    ///
    /// The bit is flipped in the **last** member — slot B's `cmdline.txt`,
    /// twenty-three bytes at the very end of the payload — which is written
    /// long after the 640 MiB system member. An installer that verified only
    /// what it was about to write next would put the whole system filesystem on
    /// the card and discover the fault afterwards, leaving a slot that is
    /// written, verified, and one file short of what was signed.
    ///
    /// So the assertion that matters is not the refusal; it is that
    /// `device_writes` is **empty**.
    #[test]
    fn a_flipped_bit_in_the_last_member_is_caught_before_the_first_is_written() {
        let mut built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let last = built.bytes.len() - 1;
        built.bytes[last] ^= 0x01;
        let mut k = card(&built.bytes);
        let before = slot_before(&k);

        let outcome = install(&mut k, false, true);
        assert!(matches!(outcome, Outcome::RefusedPayload(_)), "{outcome:?}");
        assert!(
            k.device_writes.is_empty(),
            "the system member was written before the last member had been checked: {:?}",
            k.device_writes
        );
        assert!(k.synced_writes.is_empty());
        assert_eq!(slot_before(&k), before, "the slot changed");
    }

    /// A flipped bit inside the signed header. `ADR-029` F2 — refused at §5
    /// step 7, before the manifest is even parsed.
    #[test]
    fn a_flipped_bit_in_the_signed_header_is_refused_before_anything_is_read() {
        let mut built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        // Byte 200 is inside the member table, which the signature covers.
        built.bytes[200] ^= 0x01;
        let mut k = card(&built.bytes);
        let before = slot_before(&k);

        let outcome = install(&mut k, false, true);
        assert!(matches!(outcome, Outcome::RefusedPayload(_)), "{outcome:?}");
        assert!(k.device_writes.is_empty());
        assert_eq!(slot_before(&k), before);
    }

    /// `ADR-029` §5 step 6. A perfectly valid payload signed by a key this
    /// machine does not know is refused at the table lookup.
    #[test]
    fn a_payload_signed_by_an_untrusted_key_is_refused() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        // A different key at the same path.
        let mut digest = Blake3::new();
        let other = Ed25519Signer::from_seed(&digest.of(b"somebody else"));
        k.put(KEY_PATH, other.public_key().0.to_vec());
        let before = slot_before(&k);

        let outcome = install(&mut k, false, true);
        assert!(matches!(outcome, Outcome::RefusedPayload(_)), "{outcome:?}");
        assert!(k.device_writes.is_empty());
        assert_eq!(slot_before(&k), before);
    }

    /// `ADR-029` F1 — a truncated transfer. The header names the last member's
    /// end, and a short file cannot supply it.
    #[test]
    fn a_truncated_payload_is_refused() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut short = built.bytes.clone();
        short.truncate(short.len() - 64);
        let mut k = card(&short);
        let before = slot_before(&k);

        let outcome = install(&mut k, false, true);
        assert!(matches!(outcome, Outcome::RefusedPayload(_)), "{outcome:?}");
        assert!(k.device_writes.is_empty());
        assert_eq!(slot_before(&k), before);
    }

    /// `ADR-029` F6 — the wrong board. Refused before any write, from the
    /// **signed** header.
    #[test]
    fn a_payload_for_another_platform_is_refused_before_any_write() {
        let built = build("rivet", "aarch64", "rpizw", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        let before = slot_before(&k);

        let outcome = install(&mut k, false, true);
        assert!(matches!(outcome, Outcome::RefusedPayload(_)), "{outcome:?}");
        assert!(k.device_writes.is_empty());
        assert_eq!(slot_before(&k), before);
    }

    #[test]
    fn a_payload_for_another_architecture_is_refused() {
        let built = build("rivet", "x86_64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        let outcome = install(&mut k, false, true);
        assert!(matches!(outcome, Outcome::RefusedPayload(_)), "{outcome:?}");
        assert!(k.device_writes.is_empty());
    }

    /// What slot b's record holds after an install whose system write failed:
    /// one write, of nothing, made before the slot was touched.
    fn emptied_record_of_slot_b() -> Vec<(String, Vec<u8>)> {
        vec![(
            rivet_core::deploy::slot_header_path(rivet_core::state::Slot::B),
            Vec::new(),
        )]
    }

    // -----------------------------------------------------------------------
    // Not what this machine already runs — PLAN-Feature-Set-2026-09-12 §3, found on rivet-01.
    // -----------------------------------------------------------------------

    /// The same card after it has booted the slot `card` wrote into: the
    /// firmware reports slot B and `autoboot.txt`'s default is slot B, so the
    /// boot is committed and an update would target slot A.
    fn now_running_slot_b(k: &mut FakeKernel) {
        k.bytes_mut().insert(
            DT_BOOT_PARTITION.to_string(),
            Script::always(Ok(vec![0, 0, 0, 2])),
        );
        k.files_mut().insert(
            format!("{AUTOBOOT_PARTITION_MOUNT}/autoboot.txt"),
            Script::always(Ok(
                "[all]\ntryboot_a_b=1\nboot_partition=2\n[tryboot]\nboot_partition=2\n".to_string(),
            )),
        );
        k.put(&format!("{BOOT_DISK}p3"), vec![0xdd; SYSTEM_BYTES]);
        k.declare_size(&format!("{BOOT_DISK}p3"), 640 * 1024 * 1024);
    }

    /// **The installer retains the header of what it wrote into a slot**, and
    /// only once the slot holds it — which is what makes "what the running slot
    /// holds" answerable without hashing a partition.
    #[test]
    fn an_installed_update_leaves_its_header_as_the_record_of_what_the_slot_holds() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        assert!(matches!(install(&mut k, false, true), Outcome::Installed { .. }));
        let record = rivet_core::deploy::slot_header_path(rivet_core::state::Slot::B);
        assert_eq!(
            k.device_content(&record),
            Some(&built.bytes[..HEADER_BYTES]),
            "slot b's record is not the header of the build written into it"
        );
        // Emptied first, before the system write: the order is the invariant.
        let emptied = k
            .synced_writes
            .iter()
            .position(|(path, bytes)| *path == record && bytes.is_empty())
            .expect("the record was emptied before the slot was written");
        let retained = k
            .synced_writes
            .iter()
            .rposition(|(path, bytes)| *path == record && !bytes.is_empty())
            .expect("the record was written");
        assert!(emptied < retained);
    }

    /// **The defect, as it happened on `rivet-01`.** A build installed into slot
    /// b, booted and committed, is sent again. Until 2026-09-12 it was written
    /// into slot a, replacing the rollback with a second copy of the running
    /// build. Refused now, as already running, with no device written — on a dry
    /// run as well.
    #[test]
    fn re_sending_the_build_the_running_slot_holds_is_refused_and_writes_no_slot() {
        let built = build("rivet", "aarch64", "rpi4", "0.1.85", SYSTEM_BYTES, None);
        for dry_run in [false, true] {
            let mut k = card(&built.bytes);
            assert!(matches!(install(&mut k, false, true), Outcome::Installed { .. }));
            now_running_slot_b(&mut k);
            let writes_before = k.device_writes.len();
            let slot_a_before = k.device_content(&format!("{BOOT_DISK}p3")).map(<[u8]>::to_vec);

            let outcome = install(&mut k, dry_run, true);
            let Outcome::RefusedPayload(why) = &outcome else {
                panic!("dry_run={dry_run}: the running build was installed again: {outcome:?}");
            };
            assert!(why.contains(rivet_core::deploy::ALREADY_RUNNING), "{why}");
            assert!(why.contains("slot b"), "{why}");
            assert!(why.contains("rollback"), "{why}");
            assert_eq!(k.device_writes.len(), writes_before, "a slot was written");
            assert_eq!(
                k.device_content(&format!("{BOOT_DISK}p3")).map(<[u8]>::to_vec),
                slot_a_before,
                "slot a — the rollback — was touched"
            );
        }
    }

    /// **A different build is not refused**: the running slot's record names
    /// something else, so the update proceeds into the other slot.
    #[test]
    fn a_different_build_from_the_one_the_running_slot_holds_is_installed() {
        let running = build("rivet", "aarch64", "rpi4", "0.1.85", SYSTEM_BYTES, None);
        let next = build("rivet", "aarch64", "rpi4", "0.1.86", SYSTEM_BYTES, None);
        let mut k = card(&running.bytes);
        assert!(matches!(install(&mut k, false, true), Outcome::Installed { .. }));
        now_running_slot_b(&mut k);
        k.put(PAYLOAD_PATH, next.bytes.clone());
        assert!(
            matches!(install(&mut k, false, true), Outcome::Installed { .. }),
            "a new build was refused as though it were running"
        );
        assert_eq!(
            k.device_content(&rivet_core::deploy::slot_header_path(rivet_core::state::Slot::A)),
            Some(&next.bytes[..HEADER_BYTES])
        );
    }

    /// **The same build in the slot that is NOT running is not "running".** The
    /// card installed into slot b and never booted it; a re-send targets slot b
    /// again and is not refused as already running. (Writing the same build
    /// twice into the inactive slot costs nothing a rollback needs.)
    #[test]
    fn the_build_the_inactive_slot_holds_is_not_called_running() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        assert!(matches!(install(&mut k, false, true), Outcome::Installed { .. }));
        let outcome = install(&mut k, false, true);
        assert!(matches!(outcome, Outcome::Installed { .. }), "{outcome:?}");
    }

    // -----------------------------------------------------------------------
    // Never while a trial is undecided — ADR-018 §5, PLAN-Feature-Set-2026-09-12 §3.
    // -----------------------------------------------------------------------

    /// A card whose firmware booted slot A while `autoboot.txt`'s default is
    /// slot B: an uncommitted one-shot trial of A, falling back to B.
    fn card_in_an_uncommitted_trial(payload: &[u8]) -> FakeKernel {
        let mut k = card(payload);
        k.files_mut().insert(
            format!("{AUTOBOOT_PARTITION_MOUNT}/autoboot.txt"),
            Script::always(Ok(
                "[all]\ntryboot_a_b=1\nboot_partition=2\n[tryboot]\nboot_partition=1\n".to_string(),
            )),
        );
        k
    }

    /// **The defect.** In an uncommitted trial the slot this update writes is
    /// the one a reset falls back to. Until 2026-09-12 the installer wrote it
    /// anyway. Refused now — on a dry run as well as a real one — with nothing
    /// written and the two ways forward named.
    #[test]
    fn a_system_update_is_refused_during_an_uncommitted_trial_and_writes_nothing() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        for dry_run in [false, true] {
            let mut k = card_in_an_uncommitted_trial(&built.bytes);
            let before = slot_before(&k);
            let outcome = install(&mut k, dry_run, true);
            let Outcome::RefusedMachine(why) = &outcome else {
                panic!("dry_run={dry_run}: an update ran during an uncommitted trial: {outcome:?}");
            };
            assert!(why.contains("NOT COMMITTED"), "{why}");
            assert!(why.contains("slot b"), "{why}");
            assert!(why.contains("commit"), "{why}");
            assert!(why.contains("roll back"), "{why}");
            assert!(why.contains("NOTHING WAS WRITTEN"), "{why}");
            assert!(k.device_writes.is_empty(), "dry_run={dry_run}: {:?}", k.device_writes);
            assert!(k.synced_writes.is_empty(), "dry_run={dry_run}: {:?}", k.synced_writes);
            assert_eq!(slot_before(&k), before);
        }
    }

    /// **A default that cannot be read is refused too**: whether the write would
    /// land on the slot a reset boots is then unknown.
    #[test]
    fn a_system_update_is_refused_when_the_default_slot_cannot_be_read() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        k.files_mut().insert(
            format!("{AUTOBOOT_PARTITION_MOUNT}/autoboot.txt"),
            Script::always(Err(Errno::EIO)),
        );
        let outcome = install(&mut k, false, true);
        let Outcome::RefusedMachine(why) = &outcome else {
            panic!("an update ran with the default slot unread: {outcome:?}");
        };
        assert!(why.contains("UNKNOWN"), "{why}");
        assert!(k.device_writes.is_empty());
    }

    /// **A committed trial is not refused.** `in_trial_boot` stays true for the
    /// life of a trial PID 1 has confirmed; the default is what decides, and here
    /// it is the running slot.
    #[test]
    fn a_committed_trial_takes_an_update() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        k.bytes_mut().insert(
            rivet_adapter_image::DT_TRYBOOT.to_string(),
            Script::always(Ok(vec![0, 0, 0, 1])),
        );
        assert!(
            matches!(install(&mut k, false, true), Outcome::Installed { .. }),
            "a committed trial refused an update"
        );
    }

    // -----------------------------------------------------------------------
    // The architecture, for every kind — PLAN-Feature-Set-2026-09-12 §3.
    // -----------------------------------------------------------------------

    /// **The defect.** An application built for another architecture was
    /// installed — `Application::accepts` had no caller — and failed when it
    /// ran. Refused now, with nothing written and both architectures named.
    #[test]
    fn an_application_for_another_architecture_is_refused_and_nothing_is_written() {
        let mut k = board_with(&built_application_for("photo-index", "0.1.0", "armv6"));
        let outcome = install_app(&mut k, false);
        let Outcome::RefusedPayload(why) = &outcome else {
            panic!("an armv6 application was not refused on an aarch64 machine: {outcome:?}");
        };
        assert!(why.contains("armv6"), "{why}");
        assert!(why.contains("aarch64"), "{why}");
        assert!(k.mkdirs.is_empty(), "{:?}", k.mkdirs);
        assert!(k.synced_writes.is_empty());
        assert!(k.links.is_empty());
    }

    /// **The token is the image's, not the compiler's.** A machine whose image
    /// records `armv6` installs an `armv6` application — which a comparison
    /// against `std::env::consts::ARCH` would have refused on that board, where
    /// the compiler says `arm`.
    #[test]
    fn a_machine_whose_image_records_armv6_installs_an_armv6_application() {
        let mut k = board_with(&built_application_for("photo-index", "0.1.0", "armv6"));
        k.put(
            rivet_core::package::arch::PATH,
            rivet_core::package::arch::render("armv6").expect("a token"),
        );
        let outcome = install_app(&mut k, false);
        assert!(
            matches!(outcome, Outcome::InstalledApplication { .. }),
            "{outcome:?}"
        );
    }

    /// **A machine whose image recorded no usable architecture installs
    /// nothing** — the machine's refusal, naming the file, before anything is
    /// written. The record here is the compiler's word, which is not a token.
    #[test]
    fn a_machine_whose_architecture_record_is_not_a_token_installs_nothing() {
        let mut k = board_with(&built_application("photo-index", "0.1.0"));
        k.put(rivet_core::package::arch::PATH, b"arm\n".to_vec());
        let Outcome::RefusedMachine(why) = install_app(&mut k, false) else {
            panic!("a machine whose record is not a token installed an application");
        };
        assert!(why.contains(rivet_core::package::arch::PATH), "{why}");
        assert!(why.contains("NOTHING WAS WRITTEN"), "{why}");
        assert!(k.mkdirs.is_empty(), "{:?}", k.mkdirs);
    }

    // -----------------------------------------------------------------------
    // An application shadowing an image service — defence in depth for
    // `cmd/rivet-mcpd`'s door, PLAN-Feature-Set-2026-09-12 §3,
    // `.rulings/` R-156 R10.
    // -----------------------------------------------------------------------

    /// **The defect this closes.** Before this check, an application named
    /// like `rivet-mcpd` installed, and every boot afterwards
    /// `cmd/rivet-sysd`'s own discovery refused to start it — with no
    /// uninstall able to name the shadow, because `begin_uninstall` finds the
    /// image's service of that name first. Refused here instead, before
    /// anything is written.
    #[test]
    fn an_application_named_like_an_image_service_is_refused_and_nothing_is_written() {
        let mut k = board_with(&built_application_for(
            rivet_core::service::MCPD_SERVICE_NAME,
            "0.1.0",
            "aarch64",
        ));
        let outcome = install_app(&mut k, false);
        let Outcome::RefusedPayload(why) = &outcome else {
            panic!("an application named rivet-mcpd was not refused: {outcome:?}");
        };
        assert!(why.contains(rivet_core::service::MCPD_SERVICE_NAME), "{why}");
        assert!(k.mkdirs.is_empty(), "{:?}", k.mkdirs);
        assert!(k.synced_writes.is_empty());
        assert!(k.links.is_empty());
    }

    // -----------------------------------------------------------------------
    // An application named for something this project's own machinery
    // already means — defence in depth for `cmd/rivet-mcpd`'s door,
    // `.rulings/` R-210 D21.
    // -----------------------------------------------------------------------

    /// **The same shape as the image-service test above, for the wider
    /// reserved set**: an application named `root` — a system identity, not a
    /// service — is refused here too, before anything is written.
    #[test]
    fn an_application_named_for_a_system_identity_is_refused_and_nothing_is_written() {
        let mut k = board_with(&built_application_for("root", "0.1.0", "aarch64"));
        let outcome = install_app(&mut k, false);
        let Outcome::RefusedPayload(why) = &outcome else {
            panic!("an application named root was not refused: {outcome:?}");
        };
        assert!(why.contains("root"), "{why}");
        assert!(k.mkdirs.is_empty(), "{:?}", k.mkdirs);
        assert!(k.synced_writes.is_empty());
        assert!(k.links.is_empty());
    }

    /// **`ADR-029` §4 — the whole payload, not the member.**
    ///
    /// The sharpest thing the format's design pass found: when slot A is the
    /// inactive slot, the payload's boot target is the filesystem holding the
    /// only record of which slot the firmware boots. A payload carrying
    /// `autoboot.txt` would reset the machine's default under a report of
    /// success.
    #[test]
    fn a_payload_carrying_autoboot_txt_is_refused_whole() {
        let built = build(
            "rivet",
            "aarch64",
            "rpi4",
            "0.0.2",
            SYSTEM_BYTES,
            Some(("autoboot.txt", b"[all]\nboot_partition=1\n".to_vec())),
        );
        let mut k = card(&built.bytes);
        let before = slot_before(&k);

        let outcome = install(&mut k, false, true);
        let Outcome::RefusedPayload(why) = &outcome else {
            panic!("{outcome:?}");
        };
        assert!(why.contains("autoboot.txt"), "{why}");
        assert!(k.device_writes.is_empty());
        assert_eq!(slot_before(&k), before);
    }

    /// The same refusal for the provisioning file. `ADR-004` §6 has PID 1
    /// *mutate* it and `ADR-018` §8 makes it the recovery path for a damaged
    /// `/state`; an update restoring the blank template destroys both.
    #[test]
    fn a_payload_carrying_the_provisioning_file_is_refused_whole() {
        let built = build(
            "rivet",
            "aarch64",
            "rpi4",
            "0.0.2",
            SYSTEM_BYTES,
            Some(("rivet.toml", b"[network]\n".to_vec())),
        );
        let mut k = card(&built.bytes);
        let outcome = install(&mut k, false, true);
        let Outcome::RefusedPayload(why) = &outcome else {
            panic!("{outcome:?}");
        };
        assert!(why.contains("rivet.toml"), "{why}");
        assert!(k.device_writes.is_empty());
    }

    /// `ADR-029` F8, driven through the whole binary rather than through the
    /// plan alone: a member larger than the partition is refused before
    /// anything is opened for writing.
    #[test]
    fn a_system_member_larger_than_the_partition_is_refused_before_any_write() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        k.declare_size(&format!("{BOOT_DISK}p4"), (SYSTEM_BYTES - 1) as u64);
        let before = slot_before(&k);

        let outcome = install(&mut k, false, true);
        let Outcome::RefusedPayload(why) = &outcome else {
            panic!("{outcome:?}");
        };
        assert!(why.contains("p4"), "{why}");
        assert!(k.device_writes.is_empty());
        assert_eq!(slot_before(&k), before);
    }

    /// **The test `ADR-029` §9.2 step 9 exists for**, and the only place in
    /// this project where the hazard it defends against is actually produced:
    /// a device that accepts the write, reports success, and stores something
    /// else. A loop device cannot do this and neither can a file.
    #[test]
    fn a_card_that_stores_something_else_is_caught_by_the_read_back() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        k.make_it_lie(&format!("{BOOT_DISK}p4"));

        let outcome = install(&mut k, false, true);
        let Outcome::RefusedMachine(why) = &outcome else {
            panic!("{outcome:?}");
        };
        assert!(why.contains("does not hold"), "{why}");
        // The write happened; the boot half and the arming did not.
        assert_eq!(k.device_writes.len(), 1);
        // The one file written is slot b's record, EMPTIED before the slot was
        // touched — so a slot left holding neither build is claimed by no record
        // (PLAN-Feature-Set-2026-09-12 §3).
        assert_eq!(k.synced_writes, emptied_record_of_slot_b(), "{:?}", k.synced_writes);
        // F3: the slot is left as it is. Nothing wiped it.
        assert_eq!(
            k.device_content(&format!("{BOOT_DISK}p4")).map(<[u8]>::len),
            Some(SYSTEM_BYTES)
        );
    }

    /// **The regression guard for the guard.** With `--no-read-back` the same
    /// lying card is *not* caught, the install completes, and the machine is
    /// armed to trial a slot that does not hold what was signed. That is the
    /// cost of the flag, asserted rather than described.
    #[test]
    fn without_the_read_back_the_same_lying_card_is_not_caught() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        k.make_it_lie(&format!("{BOOT_DISK}p4"));

        let outcome = install(&mut k, false, false);
        assert!(matches!(outcome, Outcome::Installed { .. }), "{outcome:?}");
        assert_ne!(
            k.device_content(&format!("{BOOT_DISK}p4")),
            Some(built.system.as_slice()),
            "the fake card did not lie, so this test proves nothing"
        );
        assert!(
            k.messages().iter().any(|m| m.contains("was not read back")),
            "the cost of --no-read-back was not said: {:?}",
            k.messages()
        );
    }

    // -----------------------------------------------------------------------
    // Skipping, the dry run, and the F5 warning.
    // -----------------------------------------------------------------------

    /// `ADR-029` §6 and §9: skip-if-identical is a **safety** property. A
    /// payload whose boot half the card already holds must not open that
    /// filesystem for writing at all.
    #[test]
    fn a_boot_half_that_is_already_identical_is_never_mounted_writable() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        for (name, bytes) in boot_invariant() {
            k.put(&format!("{PAYLOAD_BOOT_MOUNT}/{name}"), bytes);
        }
        k.put(
            &format!("{PAYLOAD_BOOT_MOUNT}/cmdline.txt"),
            b"root=PARTLABEL=system-b".to_vec(),
        );

        let outcome = install(&mut k, false, true);
        let Outcome::Installed {
            boot_written,
            boot_skipped,
            ..
        } = outcome
        else {
            panic!("{outcome:?}");
        };
        assert_eq!(boot_written, 0);
        assert_eq!(boot_skipped, 5);
        // The only writable mount left is the arming's, which is a different
        // partition and a different mount point.
        assert!(
            !k.mount_calls
                .iter()
                .any(|c| c.writable && c.at == PAYLOAD_BOOT_MOUNT),
            "the boot half was opened for writing with nothing to write: {:?}",
            k.mount_calls
        );
    }

    /// A boot half holding **slot A's** `cmdline.txt` is not identical to slot
    /// B's, so it is written. The negative half of the test above, and without
    /// it a skip that skipped everything would pass both.
    #[test]
    fn the_other_slots_cmdline_is_not_mistaken_for_this_ones() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        for (name, bytes) in boot_invariant() {
            k.put(&format!("{PAYLOAD_BOOT_MOUNT}/{name}"), bytes);
        }
        k.put(
            &format!("{PAYLOAD_BOOT_MOUNT}/cmdline.txt"),
            b"root=PARTLABEL=system-a".to_vec(),
        );

        let outcome = install(&mut k, false, true);
        let Outcome::Installed {
            boot_written,
            boot_skipped,
            ..
        } = outcome
        else {
            panic!("{outcome:?}");
        };
        assert_eq!(boot_written, 1, "only cmdline.txt should have been written");
        assert_eq!(boot_skipped, 4);
    }

    /// `ADR-006` §6.2. A dry run reaches the same verdict and writes nothing —
    /// including the arming's own check, because confirmation happens in a
    /// *second* invocation and a plan that had not looked at the card would
    /// take a confirmation and only then discover the card cannot carry it.
    #[test]
    fn a_dry_run_verifies_everything_and_writes_nothing() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        let before = slot_before(&k);

        let outcome = install(&mut k, true, true);
        assert_eq!(
            outcome,
            Outcome::Planned {
                slot: SlotId(2),
                system_bytes: SYSTEM_BYTES as u64
            }
        );
        assert_eq!(outcome.wrote_nothing(), Some(true));
        assert!(k.device_writes.is_empty());
        assert!(k.synced_writes.is_empty());
        assert_eq!(slot_before(&k), before);
        assert!(
            k.mount_calls.iter().all(|c| !c.writable),
            "a dry run mounted something writable: {:?}",
            k.mount_calls
        );
    }

    /// A dry run must reach the arming's refusals too. A card with no
    /// `tryboot_a_b=1` is `ADR-018` §5.3's trap, and discovering it after a
    /// confirmation is the failure `can_arm_trial_boot` exists to prevent.
    #[test]
    fn a_dry_run_on_a_card_that_cannot_be_armed_says_so_rather_than_verifying_and_stopping() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        k.files_mut().insert(
            format!("{AUTOBOOT_PARTITION_MOUNT}/autoboot.txt"),
            Script::always(Ok(
                "[all]\nboot_partition=1\n[tryboot]\nboot_partition=1\n".to_string()
            )),
        );
        let outcome = install(&mut k, true, true);
        let Outcome::RefusedMachine(why) = &outcome else {
            panic!("{outcome:?}");
        };
        assert!(why.contains("tryboot_a_b"), "{why}");
    }

    /// **`ADR-029` §9.3's F5, said out loud.** ~~The mitigation is unruled, so
    /// the installer must report and must not refuse~~ — **the mitigation was
    /// ruled on 2026-08-29 (`PLAN.md` §94.2) and is built**, but the record
    /// stays: detect-and-repair *narrows* the window and does not close it, so
    /// an operator still needs to know which updates run the risk. It must not
    /// be silent, because that is what would let somebody discover it on a
    /// board.
    ///
    /// **It is `info` and no longer `warn`, and this test was renamed with
    /// it.** `PLAN.md` §98: a condition that holds on every run of a normal
    /// operation is not an exception, and §94.2 measured what pretending
    /// otherwise costs — the board emitted this on every install until it
    /// **became wallpaper**. The assertion is on the *content* rather than the
    /// severity precisely so that saying it stays required while how loudly it
    /// is said remains a judgement.
    #[test]
    fn an_update_targeting_the_fallback_partition_says_so_and_still_installs() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        // Running slot B, so the update targets slot A — whose boot half is the
        // partition the firmware falls back to.
        k.bytes_mut().insert(
            DT_BOOT_PARTITION.to_string(),
            Script::always(Ok(vec![0, 0, 0, 2])),
        );
        k.put(&format!("{BOOT_DISK}p3"), vec![0xee; SYSTEM_BYTES]);
        k.declare_size(&format!("{BOOT_DISK}p3"), 640 * 1024 * 1024);
        k.files_mut().insert(
            format!("{AUTOBOOT_PARTITION_MOUNT}/autoboot.txt"),
            Script::always(Ok(
                "[all]\ntryboot_a_b=1\nboot_partition=2\n[tryboot]\nboot_partition=2\n".to_string(),
            )),
        );

        let outcome = install(&mut k, false, true);
        assert!(
            matches!(
                outcome,
                Outcome::Installed {
                    slot: SlotId(1),
                    ..
                }
            ),
            "{outcome:?}"
        );
        assert!(
            k.messages().iter().any(|m| m.contains("ADR-029 §9.3")),
            "F5 was not named at the moment it applied: {:?}",
            k.messages()
        );
    }

    // -----------------------------------------------------------------------
    // `ADR-029` §9.3's mitigation — detect and repair. Ruled 2026-08-29.
    // -----------------------------------------------------------------------

    /// The selector as a healthy card carries it, running slot B.
    const SELECTOR: &[u8] = b"[all]\ntryboot_a_b=1\nboot_partition=2\n[tryboot]\nboot_partition=2\n";

    /// A card set up so the update targets slot A — whose boot half **is** the
    /// partition the firmware falls back to, which is the only case §9.3
    /// describes.
    fn card_targeting_the_fallback(built: &[u8]) -> FakeKernel {
        let mut k = card(built);
        k.bytes_mut().insert(
            DT_BOOT_PARTITION.to_string(),
            Script::always(Ok(vec![0, 0, 0, 2])),
        );
        k.put(&format!("{BOOT_DISK}p3"), vec![0xee; SYSTEM_BYTES]);
        k.declare_size(&format!("{BOOT_DISK}p3"), 640 * 1024 * 1024);
        k.files_mut().insert(
            format!("{AUTOBOOT_PARTITION_MOUNT}/autoboot.txt"),
            Script::always(Ok(String::from_utf8(SELECTOR.to_vec()).unwrap())),
        );
        k
    }

    /// **The ordinary case, and the one that must stay quiet.** A selector that
    /// reads back identically was not disturbed, and saying so at `debug` is
    /// what makes the `warn` in the next test mean something.
    #[test]
    fn a_boot_selector_that_survives_the_write_is_reported_untouched_and_not_rewritten() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card_targeting_the_fallback(&built.bytes);
        k.bytes_mut().insert(
            format!("{PAYLOAD_BOOT_MOUNT}/autoboot.txt"),
            Script::always(Ok(SELECTOR.to_vec())),
        );

        let outcome = install(&mut k, false, true);
        assert!(
            matches!(outcome, Outcome::Installed { slot: SlotId(1), .. }),
            "{outcome:?}"
        );
        assert!(
            k.messages()
                .iter()
                .any(|m| m.contains("byte-for-byte as it did before")),
            "the selector was never checked: {:?}",
            k.messages()
        );
        // Scoped to the boot-half mount deliberately. `AUTOBOOT_PARTITION_MOUNT`
        // gets an `autoboot.txt.new` in the same install and it is **not** this:
        // that is step 9 arming the trial boot, which is a different operation
        // on a different mount point (`ADR-029` §9.1 keeps them separate). An
        // assertion that did not distinguish them would pass on the repair
        // never running.
        assert!(
            !k.synced_writes
                .iter()
                .any(|(path, _)| path.starts_with(PAYLOAD_BOOT_MOUNT)
                    && path.contains("autoboot.txt")),
            "an undamaged selector was rewritten anyway: {:?}",
            k.synced_writes
        );
    }

    /// **The hole being hit, and closed.** The selector reads good before the
    /// boot half is written and damaged after — which is exactly `ADR-029`
    /// §9.3's torn write — and the installer must put back the bytes it read
    /// first, verify them, and say so loudly.
    #[test]
    fn a_boot_selector_damaged_by_writing_the_boot_half_is_restored_from_the_bytes_read_first() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card_targeting_the_fallback(&built.bytes);
        // Read 1 is the capture on the read-only mount; read 2 is the check
        // after the writes are synced and is the damage; read 3 is the
        // verification that the repair took.
        k.bytes_mut().insert(
            format!("{PAYLOAD_BOOT_MOUNT}/autoboot.txt"),
            Script::sequence(
                vec![Ok(SELECTOR.to_vec()), Ok(b"\xff\xff\x00lorem ipsum".to_vec())],
                Ok(SELECTOR.to_vec()),
            ),
        );

        let outcome = install(&mut k, false, true);
        assert!(
            matches!(outcome, Outcome::Installed { slot: SlotId(1), .. }),
            "{outcome:?}"
        );
        assert!(
            k.messages().iter().any(|m| m.contains("HAS BEEN RESTORED")),
            "the damage was not detected or not repaired: {:?}",
            k.messages()
        );
        // The repair goes through write-sync-rename like every other write to
        // this filesystem, so what lands is the temporary name.
        assert!(
            k.synced_writes
                .iter()
                .any(|(path, bytes)| path.contains("autoboot.txt") && bytes == SELECTOR),
            "the selector was not restored to the bytes read first: {:?}",
            k.synced_writes
        );
    }

    /// **A selector that could not be read first cannot be repaired**, and the
    /// install must say that rather than reaching the end reporting nothing.
    /// `AGENTS.md` §5: an answer that omits this says the selector was fine.
    #[test]
    fn an_install_that_cannot_read_the_selector_first_says_it_cannot_repair_it() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card_targeting_the_fallback(&built.bytes);
        k.bytes_mut().insert(
            format!("{PAYLOAD_BOOT_MOUNT}/autoboot.txt"),
            Script::always(Err(Errno(2))),
        );

        let outcome = install(&mut k, false, true);
        assert!(
            matches!(outcome, Outcome::Installed { slot: SlotId(1), .. }),
            "{outcome:?}"
        );
        assert!(
            k.messages()
                .iter()
                .any(|m| m.contains("cannot detect or repair damage to it")),
            "an unreadable selector was passed over in silence: {:?}",
            k.messages()
        );
    }

    // -----------------------------------------------------------------------
    // The default-slot record's lock, and whose mount is whose.
    // -----------------------------------------------------------------------

    /// Whether `AUTOBOOT_LOCK` was held across every mount and unmount of the
    /// boot half, read off the fake's one ordered sequence.
    fn boot_half_ran_under_the_lock(k: &FakeKernel) -> bool {
        let lock = format!("lock {AUTOBOOT_LOCK}");
        let unlock = format!("unlock {AUTOBOOT_LOCK}");
        let mut held = false;
        let mut touched = false;
        for event in &k.sequence {
            if *event == lock {
                held = true;
            } else if *event == unlock {
                held = false;
            } else if event.ends_with(PAYLOAD_BOOT_MOUNT) {
                touched = true;
                if !held {
                    return false;
                }
            }
        }
        touched
    }

    /// **Slot A's boot half is the partition holding the record**, so every
    /// mount of it, the selector read and the repair run under the record's
    /// lock, taken with this binary's bound.
    #[test]
    fn writing_slot_as_boot_half_holds_the_records_lock_throughout() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card_targeting_the_fallback(&built.bytes);
        k.bytes_mut().insert(
            format!("{PAYLOAD_BOOT_MOUNT}/autoboot.txt"),
            Script::always(Ok(SELECTOR.to_vec())),
        );
        let outcome = install(&mut k, false, true);
        assert!(
            matches!(outcome, Outcome::Installed { slot: SlotId(1), .. }),
            "{outcome:?}"
        );
        assert!(boot_half_ran_under_the_lock(&k), "{:?}", k.sequence);
        assert!(
            k.lock_calls.contains(&format!(
                "lock {AUTOBOOT_LOCK} within {}",
                crate::medium::LOCK_WAIT_MS
            )),
            "{:?}",
            k.lock_calls
        );
    }

    /// Slot B's boot half is another partition and takes no lock.
    #[test]
    fn writing_slot_bs_boot_half_takes_no_lock() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        let outcome = install(&mut k, false, true);
        assert!(matches!(outcome, Outcome::Installed { .. }), "{outcome:?}");
        assert!(
            k.sequence.iter().any(|e| e.ends_with(PAYLOAD_BOOT_MOUNT)),
            "{:?}",
            k.sequence
        );
        assert!(!boot_half_ran_under_the_lock(&k), "{:?}", k.sequence);
    }

    /// **A lock another process holds past the bound refuses slot A's install
    /// before its boot half is mounted**, and nothing is written there.
    #[test]
    fn a_held_record_lock_refuses_slot_as_boot_half_before_mounting_it() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        // How many accesses of the record finish before the boot half is first
        // touched — the reads of the default slot that precede it — measured
        // on an undisturbed install, so the next lock taken, the boot half's
        // own, is the one refused below.
        let mut measured = card_targeting_the_fallback(&built.bytes);
        let _ = install(&mut measured, false, true);
        let before = measured
            .sequence
            .iter()
            .take_while(|e| !e.ends_with(PAYLOAD_BOOT_MOUNT))
            .filter(|e| e.starts_with("unlock "))
            .count();

        let mut k = card_targeting_the_fallback(&built.bytes);
        k.lock_results = Script::sequence(vec![Ok(()); before], Err(Errno::EAGAIN));
        let outcome = install(&mut k, false, true);
        assert!(
            matches!(outcome, Outcome::RefusedMachine(ref why) if why.contains(AUTOBOOT_LOCK)),
            "{outcome:?}"
        );
        assert!(
            !k.mount_calls.iter().any(|c| c.at == PAYLOAD_BOOT_MOUNT),
            "{:?}",
            k.mount_calls
        );
        assert!(
            !k.synced_writes
                .iter()
                .any(|(path, _)| path.starts_with(PAYLOAD_BOOT_MOUNT)),
            "{:?}",
            k.synced_writes
        );
    }

    /// **A mount found in place at the boot-half mount point is written
    /// through and left mounted**, and the install says so.
    #[test]
    fn an_adopted_boot_half_mount_is_never_unmounted_by_the_install() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        k.adopt_at = Some(PAYLOAD_BOOT_MOUNT.to_string());
        let outcome = install(&mut k, false, true);
        assert!(matches!(outcome, Outcome::Installed { .. }), "{outcome:?}");
        assert!(
            k.mount_calls.iter().any(|c| c.at == PAYLOAD_BOOT_MOUNT),
            "{:?}",
            k.mount_calls
        );
        assert!(
            !k.umount_calls.iter().any(|at| at == PAYLOAD_BOOT_MOUNT),
            "an adopted mount was unmounted: {:?}",
            k.umount_calls
        );
        assert!(
            k.messages().iter().any(|m| m.contains("already mounted")),
            "{:?}",
            k.messages()
        );
    }

    // -----------------------------------------------------------------------
    // The record.
    // -----------------------------------------------------------------------

    /// `ADR-028` §6.1 requires a non-newer payload to be **reported**, and
    /// `ADR-029` §12 item 7 says nothing in this project orders version
    /// strings. So both are in the record and neither is called newer.
    #[test]
    fn the_two_versions_are_reported_and_neither_is_called_newer() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.1", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        assert!(matches!(
            install(&mut k, true, true),
            Outcome::Planned { .. }
        ));
        let said = k.messages();
        assert!(
            said.iter().any(|m| m.contains("already running")),
            "{said:?}"
        );
        assert!(
            !said.iter().any(|m| m.contains("newer than")),
            "something claimed an ordering this project does not have: {said:?}"
        );
    }

    /// Every record this binary writes must be one `rivet get log` can read
    /// back. The two are a writer and a reader of one authoritative format
    /// (`ADR-022` §3), and a disagreement shows as a shorter log rather than a
    /// crash.
    #[test]
    fn every_record_an_install_writes_is_one_the_reader_accepts() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        let _ = install(&mut k, false, true);
        assert!(k.records.len() > 5, "{}", k.records.len());
        for record in &k.records {
            let line = record.to_jsonl();
            rivet_core::journal::Record::from_jsonl(&line)
                .unwrap_or_else(|e| panic!("unreadable record {line:?}: {e}"));
            assert_eq!(record.svc, crate::SELF_NAME);
        }
    }

    /// One install is one causal chain (`ADR-022` §5), so every record of a run
    /// carries the same correlation identifier.
    #[test]
    fn every_record_of_one_install_shares_a_correlation_identifier() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        let _ = install(&mut k, false, true);
        let first = k.records.first().map(|r| r.corr.clone()).expect("records");
        assert!(k.records.iter().all(|r| r.corr == first));
    }

    /// A machine with no device tree — every emulator run, and any non-Pi
    /// machine — must refuse rather than assuming a slot. `ADR-026` §4.
    #[test]
    fn with_no_device_tree_the_install_refuses_rather_than_assuming_a_slot() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        k.bytes_mut().insert(
            DT_BOOT_PARTITION.to_string(),
            Script::always(Err(crate::kernel::Errno::ENOENT)),
        );
        let outcome = install(&mut k, false, true);
        assert!(matches!(outcome, Outcome::RefusedMachine(_)), "{outcome:?}");
        assert!(k.device_writes.is_empty());
    }

    /// A card that has gone read-only mid-install — how an SD card usually
    /// announces it is dying. The system write fails, nothing is armed, and the
    /// slot is left as it is (F3).
    #[test]
    fn a_card_that_refuses_the_write_is_reported_and_nothing_is_armed() {
        let built = build("rivet", "aarch64", "rpi4", "0.0.2", SYSTEM_BYTES, None);
        let mut k = card(&built.bytes);
        k.device_write_results = Script::always(Err(crate::kernel::Errno::EROFS));
        let before = slot_before(&k);

        let outcome = install(&mut k, false, true);
        let Outcome::RefusedMachine(why) = &outcome else {
            panic!("{outcome:?}");
        };
        assert!(why.contains("p4"), "{why}");
        assert_eq!(
            k.synced_writes,
            emptied_record_of_slot_b(),
            "something beyond the emptied slot record was armed or written"
        );
        assert_eq!(slot_before(&k), before);
    }
}
