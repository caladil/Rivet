//! **The `get` verb**, and the twenty subjects this build reads.
//!
//! *(~~fifteen~~ ~~sixteen~~ ~~seventeen~~ ~~eighteen~~ ~~nineteen~~ — corrected
//! again when `fleet-root` landed beside `phones`. The
//! count in this sentence is exactly the species of claim `AGENTS.md` §7 is
//! about, and it is **derived by `tools/docs/claims.py`** from
//! [`subject_table`], which is why the number above is a bare word: the checker
//! reads it and compares. [`subject_table`] is the list; the tripwire test below
//! is what holds it.)*
//!
//! `ADR-006` §6.4's verb set is closed and the catalogue is keyed by verb, so
//! this module exists for the reason [`super::apply`]'s does: **a second
//! catalogue key would be an amendment to that section and `AGENTS.md` §1 puts
//! that out of an agent's reach.** One verb, one entry, and the subject is a
//! declared [`Input`] whose value selects the implementation.
//!
//! **[`subject_table`] is the list**, and this header deliberately no longer
//! copies it. It carried a table of three for five subjects and then for
//! thirteen, because a second copy of a list is a second thing to maintain and
//! the one nobody maintains is the one that is wrong (`ADR-011`). The subjects
//! fall into four kinds:
//!
//! | Kind | Examples |
//! | :-- | :-- |
//! | what `rivet-core` is itself authoritative for | `schema` |
//! | a file or kernel table an adapter reads | `desired-state`, `memory`, `process`, `thermal`, `boot`, `network`, `device`, `cpu`, `trust` |
//! | a computed answer over one of those | `storage`, `log`, `install`, `slot` |
//! | a path the CALLER named, rather than a fact the machine owns | `file`, `diff` — `ADR-006` §8.10 |
//! | what THIS BUILD can be asked at all | `catalogue` — `ADR-006` §8.13 |
//!
//! `ADR-006` §8.2's table once ruled out the obvious first `get` — reading the
//! desired-state document — because `ADR-014` §3.2 makes `rivet-config` depend
//! on `rivet-core` permanently, so `Document` is not a type this crate may
//! name. **That is why [`super::desired_state`] answers with the file's text
//! and not a parsed document**: the constraint was never lifted, it was
//! satisfied. `ADR-006` §6.4's `get <schema path>` is still unbuilt and is
//! §8.3's open question rather than this module's.
//!
//! # `document` became `desired-state`, 2026-08-29
//!
//! `PLAN.md` §100. `config` names a category of file; **`desired-state` names
//! the claim** — that there is one declared intent and everything else on the
//! machine is derived from it, which is `ADR-011` in a word. The prose
//! throughout the ADRs still says *"the desired-state document"*; only the
//! subject an operator types was renamed, and this project keeps **one
//! spelling** per subject so there is no alias.
//!
//! # This module was `log`'s until 2026-08-24
//!
//! `get.rs` *was* the journal query, with its catalogue entry declared beside
//! it, because there was one subject. With two, the *verb* is the thing that has
//! an implementation and the entry is declared here; the journal moved to
//! [`super::log`] unchanged. That is the same restructure `apply` took at its
//! second subject, and the two costs `apply` recorded apply here too:
//!
//! **1. The second positional input means different things to different
//! subjects** — except that here it means something to only one of them.
//! `boot` and `limit` are `log`'s; `storage` takes neither and **refuses** a
//! second word rather than ignoring it, because an operator typing
//! `rivet get storage /state` expecting `df /state` must be told that it was
//! not read.
//!
//! **2. The declared outputs are a union**, and since 2026-08-24 it is
//! *generated* rather than written here. [`subject_table`] is the one place the
//! subjects are enumerated; [`super::Operation::outputs`] merges their field
//! lists into what the catalogue publishes, and the entry also publishes each
//! subject with its own outputs, which is where the exact shapes live. No single
//! result carries every field the union names, and the union now says which
//! subject carries which without anyone maintaining the answer.
//!
//! The first is the maintainer's to overturn, and the overturn is to key the
//! catalogue by verb *and* noun, which §6.4 says is done by amending the ADR.
//! `ADR-006` §8.5 records it.

use super::machine::{Board, Observed};
use super::{ErrorCode, Input, Operation, OperationError, Results, Subject, Verb};
use super::{
    boot, catalogue, cpu, device, desired_state, fleet_root, install, log, memory, network, phones,
    process, schema, slot, status, storage, thermal, trust,
};
// `ADR-006` §8.10's two read subjects. Aliased because `diff` is also the name
// of the module that computes one, and a subject module named after a value
// module would make every `use` here ambiguous to a reader.
use super::file;
use super::file_diff as diff_subject;
use crate::value::Value;

/// **Every subject `get` reads, and what each one's result carries.**
///
/// The one place this verb's subjects are enumerated — [`super::apply::subject_table`]'s
/// reason, and the same three consumers: [`subjects`] reads the names off it,
/// [`super::Operation::outputs`] generates the published union from it, and
/// [`perform`] dispatches on the same two implementations.
pub(super) fn subject_table() -> Vec<Subject> {
    vec![
        Subject {
            name: log::SUBJECT,
            outputs: log::OUTPUTS,
        },
        Subject {
            name: storage::SUBJECT,
            outputs: storage::OUTPUTS,
        },
        // The third subject — the maintainer's install-request ruling's reporting half.
        // It reads the same journal `log` does and is not a filter over it: it
        // groups the installer's records into `ADR-022` §5's chains, which a
        // shell pipeline cannot do afterwards because the grouping depends on
        // the order the journal was appended in.
        Subject {
            name: install::SUBJECT,
            outputs: install::GET_OUTPUTS,
        },
        // The fourth, 2026-08-25 — and it is the only one that was already a
        // command before it was a subject. `rivet schema` was a bare word;
        // the maintainer ruled it into the verb it always belonged to, because
        // a bare noun is not a command and because the catalogue could not see
        // it. `crate::operation::schema` and the port carry the argument.
        Subject {
            name: schema::SUBJECT,
            outputs: schema::OUTPUTS,
        },
        // The fifth, 2026-08-26 — `ADR-032` §3.2's query socket, and the only
        // subject of this verb that reads something written down NOWHERE. The
        // other four read a file, the kernel's table or a computed value; this
        // one reads the supervisor's own memory, which is the whole
        // justification for a socket existing at all.
        // `crate::operation::status` carries the argument and the wire format.
        Subject {
            name: status::SUBJECT,
            outputs: status::OUTPUTS,
        },
        // The sixth, 2026-08-29 — `ADR-035` phase 4's first, and the one that
        // makes `apply edit` usable by a caller that cannot log in. `PLAN.md`
        // §90.3: it is the READ HALF of a write verb whose whole safety
        // property is that the caller states what it believes is there.
        Subject {
            name: desired_state::SUBJECT,
            outputs: desired_state::OUTPUTS,
        },
        // The seventh, 2026-08-29 — the first of the five the maintainer named,
        // and the one that carries `docs/design-candidates/`'s §7.4 finding:
        // pressure answers *"is this box struggling"* and a free/total ratio
        // does not.
        Subject {
            name: memory::SUBJECT,
            outputs: memory::OUTPUTS,
        },
        // The eighth, 2026-08-29 — the KERNEL's answer to what is running, which
        // `status` does not give: that one is the supervisor's own belief. Both
        // are correct and the interesting cases are where they disagree.
        Subject {
            name: process::SUBJECT,
            outputs: process::OUTPUTS,
        },
        // The ninth, 2026-08-29 — `PLAN.md` §92.3 item 1, and the one the
        // survey's §7 was written about. It is deliberately NOT `get
        // temperature`: §7.1 found that heat is the weakest of the four facts
        // and the sticky since-boot throttle flags are the strongest, because
        // they are the only one that cannot be recovered any other way.
        Subject {
            name: thermal::SUBJECT,
            outputs: thermal::OUTPUTS,
        },
        // The tenth, 2026-08-29 — `PLAN.md` §92.3 item 2, and the survey's §4:
        // *"boot id, uptime, boots in order. The journal already computes
        // `boots_in_order`."* It does, and this subject calls it rather than
        // computing a second answer to which boot came first.
        Subject {
            name: boot::SUBJECT,
            outputs: boot::OUTPUTS,
        },
        // The eleventh, 2026-08-29 — `PLAN.md` §92.3 item 2's `get slot`, and
        // the survey's §4: *"reconstructed by hand from `/proc/cmdline` twice
        // this week."* It reads TWO authorities and withholds an answer when
        // they disagree, because the answer names the partition an update
        // overwrites.
        Subject {
            name: slot::SUBJECT,
            outputs: slot::OUTPUTS,
        },
        // The twelfth, 2026-08-29 — and the one this module's own unknown-subject
        // test used to name as the word a person types from habit. `rivet-netd`
        // owns the wire and answered nothing; a board whose network is wrong was
        // debugged by logging in over the network that is wrong.
        Subject {
            name: network::SUBJECT,
            outputs: network::OUTPUTS,
        },
        // The thirteenth, 2026-08-29 — `ADR-035` §9 named
        // `/sys/class/block/*/uevent` as one of the reads a debugging session
        // actually needed, and it was reached with a shell. It is NOT
        // `storage`: a partition that failed to mount is invisible to that one
        // and present here, which is how the inactive A/B slot's `/system`
        // becomes visible at all.
        Subject {
            name: device::SUBJECT,
            outputs: device::OUTPUTS,
        },
        // The fourteenth and fifteenth, 2026-08-29 — `ADR-006` §8.10, and the only
        // subjects of this verb that read a path the CALLER named rather than a
        // fact the machine owns. `file` is the read half of `apply edit`, which
        // `edit.rs` had named against itself; `diff` is what an agent uses to
        // find out what a write would do before it writes.
        Subject {
            name: file::SUBJECT,
            outputs: file::OUTPUTS,
        },
        Subject {
            name: diff_subject::SUBJECT,
            outputs: diff_subject::OUTPUTS,
        },
        // The sixteenth, 2026-08-29 — and the one the maintainer asked for
        // FIRST. `cmd/rivet-fleet`'s monitor pane shipped the same day with a
        // section whose entire content was a sentence saying nothing on this
        // machine could answer it, which was the right thing to ship and is not
        // a thing to keep.
        //
        // **It answers counters and refuses to answer a percentage**, which is
        // the whole shape of the subject: utilisation is a RATE, and a rate is
        // two readings. See `super::cpu`.
        Subject {
            name: cpu::SUBJECT,
            outputs: cpu::OUTPUTS,
        },
        // The seventeenth, 2026-08-29 — `ADR-028` §5.5.6, which NAMED this
        // subject on 2026-08-24 as the reason the trust store is a file rather
        // than constants in a binary: *"a machine should be able to say what it
        // trusts … a constant in a stripped binary is a fact nobody can ask
        // for."* For five days the file existed and the only things that could
        // read it were the two binaries about to verify something with it.
        //
        // It is the only subject of this verb whose answer is about **what this
        // machine will accept from elsewhere** rather than about its own state,
        // and `super::trust` carries why it verifies nothing.
        Subject {
            name: trust::SUBJECT,
            outputs: trust::OUTPUTS,
        },
        // The EIGHTEENTH, 2026-08-30 — ruled by the maintainer, `.rulings/`
        // R-118, and `ADR-006` §8.13. The one subject of this verb that reads
        // NOTHING: not a file, not a kernel table, not a computed value over
        // one, and not a path the caller named. It answers what this build's
        // own `CATALOGUE` says can be asked of it.
        //
        // It exists because the machine's own refusal names `rivet --version`
        // as the authority for what exists and there is no shell on the machine
        // to run it in. See `super::catalogue` for what `tools/list` already
        // published, what it did not, and why this subject takes no port.
        Subject {
            name: catalogue::SUBJECT,
            outputs: catalogue::OUTPUTS,
        },
        // The NINETEENTH, 2026-09-27 — `ADR-061` §6, the read half of
        // `cargo rivet fleet phones`. Like `file` and `diff` it needs the
        // `Editor` port rather than `Observed`: it reads `register/phones/`
        // through the same generic path surface `get file` already reads any
        // path under `/state` with, because `rivet-core` cannot depend on
        // `rivetos-register` and so holds no opinion about a phone record's
        // TOML (`ADR-014` §3). See `super::phones` for the whole argument.
        Subject {
            name: phones::SUBJECT,
            outputs: phones::OUTPUTS,
        },
        // The TWENTIETH — `ADR-063` §4, the public certificate of the root a
        // fleet-root mode fleet server minted, and its fingerprint. It reads
        // one path it composes itself, through the `Editor` port as `phones`
        // does, and asks that port for the digest. See `super::fleet_root`.
        Subject {
            name: fleet_root::SUBJECT,
            outputs: fleet_root::OUTPUTS,
        },
    ]
}

/// Every subject `get` reads, in the order the catalogue names them.
pub fn subjects() -> Vec<&'static str> {
    subject_table().into_iter().map(|s| s.name).collect()
}

/// The catalogue entry. Declared beside the implementation of the *verb* — see
/// this module's documentation for why that is no longer a subject's module.
pub(super) const OPERATION: Operation = Operation {
    verb: Verb::Get,
    summary: "Read a fact off this machine: its journal — what it did, in the order it did it \
              (ADR-022 §9) — its storage, which is how full each mounted filesystem is \
              (ADR-005 §4.1.3), its latest install and how that ended, or the schema of the \
              configuration document it will accept (ADR-006 §7).",
    inputs: &[
        Input {
            name: "subject",
            required: true,
            several: false,
            description: "What to read. This build accepts `log`, `storage`, `install`, \
                          `schema`, `status`, `desired-state`, `memory`, `process`, `thermal`, \
                          `boot`, `slot`, `network`, `device`, `file`, `diff`, `cpu`, \
                          `trust`, `catalogue`, `phones` and `fleet-root`. \
                          START WITH `catalogue` IF YOU DO NOT KNOW WHAT THIS MACHINE \
                          CAN DO: it answers ADR-006 §6.4's whole closed verb set, \
                          which of them this build implements, and every subject of \
                          each — then `get catalogue <verb> <subject>` for what one \
                          answer contains. \
                          `cpu` answers COUNTERS rather than a utilisation, because a percentage \
                          is a rate and a rate needs two readings — ask twice and subtract. \
                          `trust` answers which signing keys this machine accepts (ADR-028 \
                          §5.5.6) and VERIFIES NOTHING: a key being listed is the first step of \
                          ADR-029 §9.2 rather than the whole of it. \
                          `phones` answers every phone this machine's FLEET SERVER has enrolled \
                          (ADR-061 §5) as RAW RECORD BYTES, hexadecimal, one per phone: this \
                          build holds no TOML reader for the register's format, so a label, a \
                          key and an enrollment time are `cargo rivet fleet phones list`'s to \
                          decode, on the workstation. \
                          `fleet-root` answers the PUBLIC certificate of the root this machine \
                          minted as a fleet-root mode fleet server (ADR-063 §4) and its SHA-256 \
                          fingerprint; `present` is false on every other machine.",
        },
        Input {
            name: "boot",
            required: false,
            several: false,
            description: "`log` ONLY, and no other subject accepts it: which boot — `latest` \
                          (the default, this one), `previous` (the one before it — what to ask \
                          after a board came back), `all`, or a boot identifier as it appears in \
                          the records. A WORD THAT IS NONE OF THOSE IS REFUSED, and the refusal \
                          names the boots this journal holds: THIS IS NOT A SERVICE FILTER, and \
                          `get log` has none — put a service name here and you are selecting a \
                          boot. Select on each record's `svc` field instead. \
                          The journal cannot be selected by TIME: a Pi has no RTC, \
                          so every boot starts at the epoch until something corrects it and the \
                          wall clock jumps when it does (ADR-021, ADR-022 §3.2). `storage` takes \
                          no second word and refuses one rather than ignoring it. `file` and \
                          `diff` REQUIRE this position and mean something else by it: the \
                          ABSOLUTE path to read (ADR-006 §8.10). That is §8.5.1's polymorphic \
                          positional input, paid again.",
        },
        Input {
            name: "limit",
            required: false,
            several: false,
            description: "`log` ONLY: keep only the last N records after `boot` has selected. A \
                          journal is read from the end. NAMING NOTHING IS NOT THE SAME AS \
                          ASKING FOR EVERYTHING — a caller who omits this gets the last 200 \
                          (ADR-022 §9.6), and the word `all` is how to ask for the whole \
                          journal, which ADR-022 §8 caps at eight megabytes. `diff` REQUIRES \
                          this position and means something else by it: the WHOLE TEXT to \
                          compare the file against, as one word (ADR-006 §8.10.6). \
                          `file` means a third thing by it: the BYTE OFFSET to start \
                          reading at, digits only. One answer carries at most a mebibyte \
                          whatever is asked for, so this is how a larger file is read — \
                          ask again at `content_offset + content_bytes` until \
                          `content_complete` is true.",
        },
        Input {
            name: "since",
            required: false,
            several: false,
            description: "`log` ONLY, and the FOURTH word: keep only what happened \
                          AFTER a position the machine handed you. Every `log` answer carries \
                          a `cursor`; pass the last one back and repeated calls become a tail \
                          (ADR-035 §4A). PASS IT BACK VERBATIM — it is opaque, and a value a \
                          caller composed is refused rather than interpreted. THIS IS NOT A \
                          FOLLOW: a follow is the MACHINE continuing to answer, and this is \
                          the caller asking again, which is a different thing and is named \
                          differently on purpose. If the boot your cursor names has been \
                          dropped by retention, the answer holds EVERYTHING and says so in \
                          `since` — repeated records are visible to you, a silent gap would \
                          not be. \
                          `file` means something else by this position: how many bytes to \
                          read from the offset named in the third. Omitting it reads as much \
                          as one answer may carry, which is what a caller reading a large \
                          file in pieces wants; naming one is for a caller that wants a \
                          specific span, such as a header.",
        },
    ],
    results: Results::PerSubject(subject_table),
    errors: &[
        ErrorCode::UnknownSubject,
        ErrorCode::Arguments,
        ErrorCode::MachineRefused,
    ],
};

/// Whether every key a result carries appears among `declared`, **in
/// `declared`'s order**.
///
/// A **subsequence** rather than an equality, because both subjects omit fields
/// rather than emitting them empty: `crate::value` has no null, so absence is
/// how a value says it has no such thing.
///
/// It is here rather than in either subject because both need it and a second
/// copy is `ADR-011` in miniature. Both call it from a `debug_assert!` in the
/// code path that builds the result — rather than only from a test — for
/// [`super::trial`]'s reason: the catalogue naming a field no result carries is
/// `ADR-006` §7's *"trusted and wrong"*.
pub(super) fn in_declared_order<'a>(
    fields: &[(&'static str, Value)],
    declared: impl IntoIterator<Item = &'a str>,
) -> bool {
    let declared: Vec<&str> = declared.into_iter().collect();
    let mut at = 0usize;
    for (key, _) in fields {
        match declared.iter().skip(at).position(|d| d == key) {
            Some(found) => at += found + 1,
            None => return false,
        }
    }
    true
}

/// Perform `get`. Arity is [`super::perform`]'s; the subject selects the
/// implementation and nothing else here decides anything.
/// **It takes the whole board rather than one port**, since 2026-08-29.
///
/// Every subject before `ADR-006` §8.10 read a fact the machine owns, so
/// [`Observed`] was the whole of what this verb needed. `get file` and
/// `get diff` read a path the *caller* named, which is
/// [`super::machine::Editor`]'s port — the same one `apply edit` writes
/// through, deliberately, because two readers of one file are two answers about
/// whether it could be read (`ADR-011`).
///
/// So the dispatcher takes the object every adapter already has and narrows at
/// each call, which is `super::apply::perform`'s shape and its argument: `get
/// log` still cannot write a file, and `get file` still cannot reboot anything.
pub(super) fn perform(
    arguments: &[&str],
    board: &mut dyn Board,
) -> Result<Value, OperationError> {
    let subject = arguments.first().copied().unwrap_or_default();

    // **The catalogue gates the dispatch**, for the reason `super::apply`'s does
    // and after the defect that one shipped for a commit on 2026-08-26: a
    // subject with an arm and no table entry is reachable, absent from
    // `rivet --version`, and absent from the generated output union. One
    // `contains` makes the claim this module's documentation has always made
    // true in both directions.
    if !subjects().contains(&subject) {
        return Err(unknown(subject));
    }

    // **The one subject that reads nothing at all**, first because it needs no
    // part of the board and taking one would say otherwise. `super::catalogue`
    // carries the argument: the catalogue is this crate's own `const`, and a
    // port for it would be a port an adapter could answer untruthfully about
    // what this dispatcher does.
    if subject == catalogue::SUBJECT {
        return catalogue::perform(arguments);
    }

    // `ADR-006` §8.10's two, first because they are the two that need the
    // `Editor` port — everything below this line narrows to `Observed`.
    if subject == file::SUBJECT {
        return file::perform(arguments, board);
    }
    if subject == diff_subject::SUBJECT {
        return diff_subject::perform(arguments, board);
    }
    // `ADR-061` §6 — the third subject that needs the `Editor` port rather
    // than `Observed`, for `super::phones`'s reason: rivet-core cannot depend
    // on rivetos-register, so it reads a phone record's bytes the same way
    // `get file` reads any other path.
    if subject == phones::SUBJECT {
        return phones::perform(arguments, board);
    }
    // `ADR-063` §4 — the fourth: it reads its composed path and asks the
    // same port for the digest.
    if subject == fleet_root::SUBJECT {
        return fleet_root::perform(arguments, board);
    }
    let observed: &mut dyn Observed = board;

    if subject == log::SUBJECT {
        return log::perform(arguments, observed);
    }
    if subject == storage::SUBJECT {
        return storage::perform(arguments, observed);
    }
    if subject == schema::SUBJECT {
        return schema::perform(arguments, observed);
    }
    if subject == install::SUBJECT {
        return install::perform_get(arguments, observed);
    }
    if subject == status::SUBJECT {
        return status::perform(arguments, observed);
    }
    if subject == desired_state::SUBJECT {
        return desired_state::perform(arguments, observed);
    }
    if subject == memory::SUBJECT {
        return memory::perform(arguments, observed);
    }
    if subject == process::SUBJECT {
        return process::perform(arguments, observed);
    }
    if subject == thermal::SUBJECT {
        return thermal::perform(arguments, observed);
    }
    if subject == boot::SUBJECT {
        return boot::perform(arguments, observed);
    }
    if subject == slot::SUBJECT {
        return slot::perform(arguments, observed);
    }
    if subject == network::SUBJECT {
        return network::perform(arguments, observed);
    }
    if subject == device::SUBJECT {
        return device::perform(arguments, observed);
    }
    if subject == cpu::SUBJECT {
        return cpu::perform(arguments, observed);
    }
    if subject == trust::SUBJECT {
        return trust::perform(arguments, observed);
    }
    // Unreachable through the gate above. The same refusal rather than an
    // `unreachable!`, for `super::perform`'s reason: a panic here would be a
    // panic in pid 1's dependency tree.
    Err(unknown(subject))
}

/// **The one refusal for a subject this build does not read**, naming the whole
/// vocabulary. Two callers — the gate and its unreachable tail — and one
/// sentence, which is `ADR-011` §3 in the small.
fn unknown(subject: &str) -> OperationError {
    OperationError::about(
        ErrorCode::UnknownSubject,
        subject,
        format!(
            "`{}` reads {} in this build. The desired-state `get <schema path>` of ADR-006 §6.4 \
             is not built — ADR-006 §8.3 is the open question, not an omission. \
             `rivet --version` publishes the operation catalogue, which is generated from the \
             types and is the authority for what exists.",
            Verb::Get,
            subjects()
                .iter()
                .map(|s| format!("`{s}`"))
                .collect::<Vec<_>>()
                .join(" and "),
        ),
    )
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::operation::machine::{JournalRead, MachineError, StorageRead};

    /// A machine that answers nothing, because **an unknown subject must be
    /// refused before anything is read.**
    ///
    /// Every method panics rather than returning an error: a stand-in that
    /// refused politely would let a dispatch defect pass as a refusal with a
    /// different code, and `AGENTS.md` §3 wants a stub that cannot be mistaken
    /// for the thing it stands in for.
    struct NoMachine;

    // Since 2026-08-29 this verb dispatches on the whole `Board` — `ADR-006`
    // §8.10's two subjects read through the `Editor` port. The other three
    // refuse, so a dispatch test that started editing, installing or rebooting
    // fails loudly rather than passing on a fiction (`AGENTS.md` §3).
    crate::operation::machine::refuses_to_manage!(NoMachine);
    crate::operation::machine::refuses_to_deploy!(NoMachine);
    crate::operation::machine::refuses_to_edit!(NoMachine);

    impl Observed for NoMachine {
        /// **Refuses rather than answering that this machine trusts nobody.**
        /// An empty trust store is a REAL reading -- it is what `ADR-028` §5.5.5's
        /// `--no-trust` image produces -- and this fake has not taken it. The
        /// direction is the dangerous one: *"trusts nobody"* is the answer
        /// somebody acts on by re-signing an artifact.
        fn read_trust(&mut self) -> Result<crate::operation::machine::TrustRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than answering an idle machine.** Zeroed counters
        /// are what a machine that has run nothing since it booted looks like,
        /// and a load average of zero is what an idle one looks like — both are
        /// readings, and this fake has taken neither (`AGENTS.md` §3, §5).
        fn read_cpu(&mut self) -> Result<crate::operation::machine::CpuRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than answering that this machine has no block
        /// devices**, which is not a state a machine that booted off one can
        /// be in.
        fn read_devices(&mut self) -> Result<crate::operation::machine::DeviceRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than answering an empty network.** No interfaces at
        /// all reads as a machine with none, which is not a state a running
        /// Linux is in.
        fn read_network(&mut self) -> Result<crate::operation::machine::NetworkRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than naming a slot.** Naming the wrong one is how
        /// an update overwrites the running system, so a fake that observes
        /// nothing names none.
        fn read_slots(&mut self) -> Result<crate::operation::machine::SlotRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than answering an uptime of zero.** `0` would say
        /// this machine booted a moment ago, which is a reading nobody took.
        fn read_boot(&mut self) -> Result<crate::operation::machine::BootRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than answering with unset flags.** Reporting a
        /// `Throttle` of all-`false` would assert that this machine has never
        /// been under-volted since it booted — the one claim this subject must
        /// never make without a measurement (`AGENTS.md` §3, §5).
        fn read_thermal(
            &mut self,
        ) -> Result<crate::operation::machine::ThermalRead, MachineError> {
            Err(MachineError::Refused(
                    "not this fake's port; unread throttle flags are never reported as false"
                        .to_string(),
            ))
        }
    fn read_processes(
        &mut self,
    ) -> Result<crate::operation::machine::ProcessRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    fn read_memory(&mut self) -> Result<crate::operation::machine::MemoryRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    fn read_desired_state(&mut self) -> Result<Option<String>, MachineError> {
        Ok(None)
    }
        /// `ADR-032` §3.2's query socket, added 2026-08-26. This fake asks no
        /// supervisor and refuses rather than pretending (`AGENTS.md` §3).
        fn supervisor_status(
            &mut self,
        ) -> Result<crate::operation::status::Status, MachineError> {
            Err(MachineError::Refused("this fake answers no supervisor".to_string()))
        }

        fn read_journal(&mut self) -> Result<JournalRead, MachineError> {
            panic!("an unknown subject read the journal")
        }
        /// The fake supplies no schema: `rivet-core` cannot name one
        /// (`ADR-014` §3.2), so a test that needs the real document belongs in
        /// the adapter that has both crates. Loud rather than empty, so an
        /// assertion against real schema content fails instead of passing.
        fn configuration_schema(&mut self) -> Value {
            Value::str("<no schema: this is a test fake>")
        }

        fn read_filesystems(&mut self) -> Result<StorageRead, MachineError> {
            panic!("an unknown subject read the mount table")
        }
    }

    // The two tests that stood here until 2026-08-24 — that every subject's
    // fields are a subsequence of the published union, and that nothing is
    // published no subject carries — are gone because the union is no longer
    // written down. `super::Operation::outputs` generates it from
    // [`subject_table`], so both properties are now facts about the generator
    // rather than about this module, and
    // `every_subjects_fields_survive_the_generated_union` in `super` checks them
    // once for the whole catalogue instead of once per verb.

    /// The subject list is generated from the implementations, so a subject that
    /// exists and is not listed cannot happen — and neither can the reverse.
    #[test]
    fn every_listed_subject_is_reachable_and_none_of_them_is_listed_twice() {
        let listed = subjects();
        assert_eq!(
            listed,
            vec![
                "log", "storage", "install", "schema", "status", "desired-state", "memory",
                "process", "thermal", "boot", "slot", "network", "device", "file", "diff", "cpu",
                "trust",
                "catalogue",
                "phones",
                "fleet-root"
            ]
        );
        let mut seen: Vec<&str> = Vec::new();
        for word in &listed {
            assert!(!seen.contains(word), "{word} is listed twice");
            seen.push(word);
        }
    }

    /// The catalogue names every subject, so an operator who ran a wrong one is
    /// told the whole vocabulary rather than only that theirs was wrong.
    #[test]
    fn the_operations_own_description_lists_every_subject_it_accepts() {
        let described = OPERATION.inputs[0].description;
        for subject in subjects() {
            assert!(
                described.contains(subject),
                "{subject} is not described: {described}"
            );
        }
    }

    /// A subject this build does not read is refused **by name**, and the
    /// refusal lists what it does read — so the next attempt can be right
    /// without reading the source.
    #[test]
    fn an_unknown_subject_is_refused_and_names_every_subject_this_build_reads() {
        // ~~`network` is the one a person types from habit and the one ADR-006
        // §6.4's schema-path shape will eventually make valid, so it must be an
        // unknown SUBJECT rather than anything that reads as permanent.~~
        //
        // **`network` became a real subject on 2026-08-29** — struck rather than
        // deleted, because the comment predicted its own expiry and was right.
        // `ip` replaces it as the word a person types from habit, and the
        // property being asserted is unchanged: none of these reads as
        // permanently impossible.
        for word in ["ip", "config", "", "logs", "df"] {
            let refusal = perform(&[word], &mut NoMachine)
                .expect_err(&format!("`get {word}` answered rather than refusing"));
            assert_eq!(refusal.code(), ErrorCode::UnknownSubject, "{word:?}");
            assert_eq!(refusal.subject(), Some(word));
            for subject in subjects() {
                assert!(
                    refusal.message().contains(subject),
                    "{word:?}: {subject} missing from {}",
                    refusal.message()
                );
            }
        }
    }
}
