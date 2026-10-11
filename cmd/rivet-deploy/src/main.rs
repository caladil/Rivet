//! `rivet-deploy` — **the installer, and it is its own supervised binary.**
//!
//! The maintainer's 2026-08-24 ruling. The recommendation was `cmd/rivet`;
//! the maintainer took a third option, and the argument that decided it was the
//! caveat rather than the case:
//!
//! > **an update writes 640 MiB, and doing that from the login shell means
//! > a session owns a long write.** A connection that drops mid-write owns a
//! > half-written slot.
//!
//! So the installer is its own supervised thing. `ADR-029` §9.2 is the sequence
//! it performs and `ADR-029` §12 item 4 is the question it answers.
//!
//! # The shape of this crate
//!
//! | Module | Holds |
//! | :-- | :-- |
//! | [`args`] | what `argv` asked for, and **the two things the maintainer's 2026-08-24 ruling left unruled** |
//! | [`journal`] | `ADR-022` §3.2's record, stamped for this binary |
//! | [`kernel`] | every operating-system fact, behind a port |
//! | [`syscall`] | the three calls `std` does not have. The only `unsafe` here |
//! | [`medium`] | `rivet-adapter-image`'s `BootMedium` over that port — **`ADR-029` §12 item 4's second half** |
//! | [`install`] | `ADR-029` §9.2, performed. The order, and nothing else |
//! | `testkernel` | a `Kernel` in a `BTreeMap`, under `cfg(test)` |
//! | `bench` | the fixture generator for the end-to-end run, `#[ignore]`d |
//!
//! The **decisions** are not here at all. Which member goes to which target, in
//! what order, and which may be skipped is `rivet_adapter_image::install`;
//! whether the bytes are the ones the signature covers is
//! `rivet_core::package`; the cryptography is `rivet-crypto`. This crate is
//! the composition root and the I/O.
//!
//! # What has run, and what has not — `AGENTS.md` §6
//!
//! **`ADR-029` §14.6 and `PLAN.md` §0.10 are the account.** Read those rather
//! than this paragraph, which is a dated observation about a tree.
//!
//! In one line: **a real signed payload has reached a real inactive slot on a
//! loop device over a copy of a real card image**, in both slot directions, with
//! the corrupted and refused cases watched leaving the slot byte-identical.
//!
//! **Nothing in this crate has run on a Raspberry Pi**, and nothing it writes
//! has reached an SD card. `ADR-026` §4 puts `/dev/mmcblk0`, `tryboot` and
//! `/proc/device-tree/chosen/bootloader` in the column QEMU does not emulate at
//! all, so an install cannot be performed under the emulator either: a guest
//! has no slots to write. A loop device is not a card — it cannot produce the
//! one hazard `ADR-029` §9.2 step 9 exists for, and neither can it go read-only
//! mid-write.
//!
//! # It installs and arms. It does not reboot
//!
//! `ADR-029` §9.1, ruled 2026-08-23: an armed-but-not-rebooted machine is
//! **completely inert**, because on the Pi the one-shot lives in the reboot call
//! rather than in the file. So install-now-reboot-later already works with no
//! marker and no new authority, and this binary stops at the arming and says
//! which kind of reboot carries it. The reboot is `cmd/rivet`'s
//! (`ADR-017` §3.1.10 — PID 1 performs a trial restart on `SIGUSR1`).

mod args;
/// The end-to-end bench's fixture generator. `#[ignore]`d, and the reason it
/// exists is `AGENTS.md` §6 — see the module.
#[cfg(test)]
mod bench;
mod install;
mod journal;
mod kernel;
mod medium;
mod syscall;
#[cfg(test)]
mod testkernel;

/// `ADR-017` §2.1's name for this service. Also the `svc` column of every record
/// this binary writes (`ADR-022` §3.2), and what a service definition would call
/// it.
///
/// ~~A literal.~~ **`rivet_core::deploy::SERVICE_NAME` since 2026-08-24**, when
/// three other crates acquired an opinion about this word at once: the
/// supervisor's service definition, the image's install path, and
/// `rivet get install`'s filter over the journal. A binary crate has no
/// library target, so none of the four could have owned it — `ADR-011`, and
/// `cmd/rivet-sysd`'s deleted path constant is the worked example of
/// what the second spelling costs.
pub const SELF_NAME: &str = rivet_core::deploy::SERVICE_NAME;

/// `ADR-022` §5's actor, composing with `ADR-012` §4's provenance.
///
/// **`update` and not the operator who asked**, and the gap is worth naming
/// rather than papering over: `ADR-028` §4 rules that *authority* comes from an
/// authenticated principal, and `ADR-012` §4 requires actor provenance from the
/// start. Nothing carries a principal into this process today — `argv` has no
/// place for one and the maintainer's 2026-08-24 ruling left the channel unruled — so the honest
/// actor is this process acting on the instruction it was given, and a record
/// naming a person would be inventing one.
///
/// Moved to `rivet_core::deploy::ACTOR` on 2026-08-24, for [`SELF_NAME`]'s
/// reason: `ADR-022` §9's reader filters on this word and the reader is in
/// another crate.
pub const ACTOR: &str = rivet_core::deploy::ACTOR;

/// `ADR-027` §3's `<product>` token, which `ADR-029` §9.2 step 2 compares.
///
/// ~~**A fourth spelling**, and it is recorded rather than quietly added … The
/// one place it should live is `rivet_core::package`, which is a library and
/// can be the authority; this crate is a binary and could never have been.~~
///
/// **MIGRATED 2026-08-24.** It lives there now and this is a re-export, not a
/// spelling. The comment above is struck rather than deleted because it is the
/// argument that produced the move, and `AGENTS.md` §7 wants the record to show
/// its own history.
///
/// **Re-exported rather than imported at each use** so that a reader of this
/// binary still finds the token where they expect it, and follows one hop to the
/// authority instead of finding a second definition (`ADR-011`).
pub use rivet_core::package::update::PRODUCT;

/// Exit codes. `ADR-017` §3.5 restarts a failed service with backoff, so these
/// are policy rather than decoration — and `ADR-017` §4.3's run-once shape means
/// **0 is what makes the supervisor leave this service alone** (`Done`) while
/// anything else is a `Backoff` and a restart.
///
/// That is the reason [`REFUSED`](exit::REFUSED) and [`BAD_ARGUMENTS`](exit::BAD_ARGUMENTS) are distinct from
/// [`MACHINE`](exit::MACHINE): a payload this machine will not install is not a *temporal*
/// failure (`ADR-017` §3.5.1) and restarting will not fix it, whereas a card
/// that was busy might. The supervisor cannot act on the difference today —
/// §4.3 records that making it understand an exit-code table is a coupling this
/// project refused — so the distinction is for the operator and the journal.
mod exit {
    /// Installed and armed, or a dry run that verified.
    pub const OK: i32 = 0;
    /// The payload is not one this machine will install, and **nothing was
    /// written**.
    pub const REFUSED: i32 = 1;
    /// `argv` was not usable.
    pub const BAD_ARGUMENTS: i32 = 2;
    /// The machine, the card or the kernel refused.
    pub const MACHINE: i32 = 3;
}

fn main() {
    use rivet_adapter_image::BootSlots as _;
    use rivet_core::journal::{Severity, Value};

    let argv: Vec<String> = std::env::args().skip(1).collect();
    let options = match args::parse(&argv) {
        Ok(args::Request::Usage) => {
            print!("{}", args::usage());
            return;
        }
        Ok(args::Request::Install(options)) => options,
        Err(why) => {
            // Before the journal exists, so it is a plain line on stderr rather
            // than a record: a record needs a boot id and a wall clock, and this
            // failure happens before either is worth reading.
            eprintln!("{SELF_NAME}: {why}");
            eprint!("{}", args::usage());
            std::process::exit(exit::BAD_ARGUMENTS);
        }
    };

    let mut k = kernel::StdKernel::new(options.form);
    let boot = boot_identifier(&mut k);
    let salt = salt(&mut k);
    let mut journal = journal::Journal::new(boot, salt, ACTOR.to_string());
    let corr = journal.next_correlation();

    let machine = install::Machine {
        product: PRODUCT.to_string(),
        // No `arch`: the image records it and `install::run` reads it, because
        // the compiler's word is `arm` where `ADR-027` §3's is `armv6` — see
        // `install::Machine`.
        //
        // `ADR-027` §3.1: `<platform>` answers "will this firmware boot it", so
        // it comes from the boot port rather than from a constant here.
        platform: rivet_adapter_image::PiFirmware
            .platform_token()
            .to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };

    let mut digest = rivet_crypto::Blake3::new();
    let verifier = rivet_crypto::Ed25519Verifier::new();
    let request = install::Request {
        payload: &options.payload,
        trusted_keys: &options.trusted_keys,
        dry_run: options.dry_run,
        read_back: options.read_back,
    };

    let outcome = install::run(
        &mut k,
        &rivet_adapter_image::PiFirmware,
        &mut journal,
        &corr,
        &machine,
        &request,
        &mut digest,
        &verifier,
    );

    // **Every arm below emits exactly one record and every one of them carries
    // `outcome`.** Added 2026-08-24, with `rivet get install`.
    //
    // The token is `rivet_core::deploy::Outcome`'s and the reason it exists is
    // that ADR's own (`ADR-006` §4.1, applied to a record rather than to a
    // refusal): *an agent branches on the code; a human reads the string.*
    // `get install` has to answer **did this install finish, and how**, and the
    // only thing that said so before was the English in `msg` — so the reader
    // would have matched on prose, and the day somebody improved a sentence
    // every finished install would silently have started reporting itself as
    // still running. `AGENTS.md` §7 is that failure exactly.
    //
    // **It is a name for what the last line already said**, not a protocol.
    // Nothing about the *middle* of an install is described anywhere, because a
    // stage vocabulary would be a protocol and that is not an agent's to
    // decide (`AGENTS.md` §1).
    //
    // The two plan arms emitted **nothing** until this change, so a dry run's
    // chain simply stopped. That is the one behaviour change here: a dry run now
    // ends with a record saying it was a dry run, which is what stops
    // `get install` reporting a completed `--dry-run` as an install still in
    // flight.
    let outcome_field = journal::field(
        rivet_core::deploy::OUTCOME_FIELD,
        Value::Str(outcome.ended().as_str().to_string()),
    );
    let code = match &outcome {
        install::Outcome::Installed { armed, .. } => {
            journal.emit(
                &mut k,
                Severity::Info,
                &corr,
                "the install is finished and the slot is armed. This binary does not \
                 reboot — ADR-029 §9.1",
                vec![
                    outcome_field,
                    journal::field("reboot", Value::Str(armed.to_string())),
                ],
            );
            exit::OK
        }
        install::Outcome::InstalledApplication { name, version, .. } => {
            journal.emit(
                &mut k,
                Severity::Info,
                &corr,
                "the application is installed. This binary does not START it — the \
                 supervisor does, on its next convergence, and only if the document \
                 enables it (ADR-015 §3.2.1)",
                vec![
                    outcome_field,
                    journal::field("app_name", Value::Str(name.clone())),
                    journal::field("app_version", Value::Str(version.clone())),
                ],
            );
            exit::OK
        }
        // Shared Rivets, 2026-08-31.
        install::Outcome::InstalledLibrary { reference, files } => {
            journal.emit(
                &mut k,
                Severity::Info,
                &corr,
                "the library is installed. NOTHING RUNS A LIBRARY — it is there for the \
                 applications that declare this reference in their closure",
                vec![
                    outcome_field,
                    journal::field("reference", Value::Str(reference.clone())),
                    journal::field("files", Value::Int(*files as i64)),
                ],
            );
            exit::OK
        }
        install::Outcome::LibraryAlreadyInstalled { reference } => {
            journal.emit(
                &mut k,
                Severity::Info,
                &corr,
                "this library was already on this machine, byte for byte, so nothing was \
                 written. A library's directory is named by a digest of its own contents, \
                 so a directory that exists holds exactly these bytes",
                vec![
                    outcome_field,
                    journal::field("reference", Value::Str(reference.clone())),
                    journal::field("wrote", Value::Bool(false)),
                ],
            );
            exit::OK
        }
        install::Outcome::Planned { .. }
        | install::Outcome::PlannedApplication { .. }
        | install::Outcome::PlannedLibrary { .. } => {
            journal.emit(
                &mut k,
                Severity::Info,
                &corr,
                "this was a DRY RUN. Everything above was verified and planned and NOTHING \
                 WAS WRITTEN (ADR-006 §6.2), so the machine is exactly as it was. Run the \
                 same command without --dry-run to perform it",
                vec![outcome_field, journal::field("wrote", Value::Bool(false))],
            );
            exit::OK
        }
        install::Outcome::RefusedPayload(why) => {
            journal.emit(
                &mut k,
                Severity::Error,
                &corr,
                &format!("the payload is refused: {why}"),
                vec![outcome_field, journal::field("wrote", Value::Bool(false))],
            );
            exit::REFUSED
        }
        install::Outcome::RefusedMachine(why) => {
            // **No `wrote` field**, and its absence is the correction rather
            // than an omission. This said `wrote: false` on every machine
            // refusal until 2026-08-24 — including one that arrived part-way
            // through a 640 MiB write, which is the only case in this binary
            // where an inactive slot is left neither old nor new. Asserting an
            // untouched slot there is `AGENTS.md` §5's failure pointing the
            // dangerous way. `install::Outcome::wrote_nothing` now answers
            // `None` for this arm and `crate::journal`'s `Value` has no null,
            // so the honest form is to say nothing (`ADR-006` §8.6.3).
            let mut fields = vec![outcome_field];
            if let Some(nothing) = outcome.wrote_nothing() {
                fields.push(journal::field("wrote", Value::Bool(!nothing)));
            }
            journal.emit(
                &mut k,
                Severity::Error,
                &corr,
                &format!(
                    "the install did not complete: {why}. WHETHER ANYTHING WAS WRITTEN IS NOT \
                     RECORDED HERE, because this refusal can arrive before the first byte or \
                     part-way through a 640 MiB write; the records above this one are what say \
                     how far it got. The inactive slot is never the running one, so this \
                     machine still boots what it booted"
                ),
                fields,
            );
            exit::MACHINE
        }
    };
    std::process::exit(code);
}

/// `ADR-022` §3.2's `boot` field, from the kernel that owns the fact.
///
/// ~~The same path `cmd/rivet-sysd` and `cmd/rivet-timed` read.~~ The same
/// path, yes — and until 2026-08-24 **not the same value**: this function kept
/// the first eight characters while the supervisor kept the whole UUID, so one
/// journal carried two names for one boot and `rivet get log previous`
/// selected wrongly. Both now call `rivet_core::journal::boot_identifier`,
/// which is where the rule lives (`ADR-011`).
fn boot_identifier(k: &mut dyn kernel::Kernel) -> String {
    rivet_core::journal::boot_identifier(
        k.read_file(rivet_core::journal::BOOT_ID_PATH).map_err(|_| ()),
    )
}

/// Separates two runs of this binary within one boot — see
/// [`journal::Journal::next_correlation`].
#[allow(clippy::cast_possible_truncation)]
fn salt(k: &mut dyn kernel::Kernel) -> u32 {
    // The monotonic clock, which is monotone within a boot and therefore cannot
    // repeat for two runs that are not simultaneous.
    k.now_ms() as u32
}
