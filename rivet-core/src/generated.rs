//! What Rivet generates **outside `/etc`**, so `explain` can answer about it.
//!
//! # Why this module exists, and what it cost not to have it
//!
//! `crate::operation::explain` shipped knowing exactly one class of path —
//! [`crate::etc::ETC_FILES`], the runtime `/etc` projection — and answering
//! *"Rivet does not generate `<path>`"* about everything else. That sentence
//! is not a shrug: it is a **claim**, and for at least one path it was false in
//! the most expensive possible direction.
//!
//! It said so about every path this module now declares — each of them a file
//! the supervisor **rewrites from the stored document on every boot**. An
//! operator who believed that sentence would edit one by hand, watch it work,
//! and lose it at the next reboot — on a headless board whose only other
//! channel is a card written at the desk. That is the one failure mode this
//! whole project is arranged to avoid, arriving through the operation whose
//! entire job is to prevent it.
//!
//! So the rule this module enforces is narrow and absolute: **`explain` may say
//! "Rivet does not generate this" only about paths Rivet really does not
//! generate.** Everything `rivet-core` is authoritative for is declared here.
//!
//! # Two sources, not one, and that is deliberate
//!
//! `ADR-011` §2 — one thing owns each fact, and the system says which one at
//! the moment the answer is needed. `explain` therefore composes rather than
//! copies:
//!
//! | Class | Declared in | Folded in by |
//! | :-- | :-- | :-- |
//! | the `/etc` projection | [`crate::etc::ETC_FILES`] | `explain` |
//! | everything else Rivet writes | [`GENERATED_PATHS`], below | this module |
//!
//! The first row is the shape to notice. Those files already have an owner, a
//! mode and a one-sentence reason, all carried as data; copying them here would
//! be the second spelling `ADR-011` §3 names, and it would go stale the first
//! time one is added. `explain` reads the other table directly instead, so an
//! entry added there becomes explainable with no change in this file.
//!
//! # What is deliberately absent, and why it is not an oversight
//!
//! **`/state/clock`.** `ADR-021` §3's persisted time lives at a path
//! `rivet-store` owns, and `ADR-014` §3.2 makes `rivet-core` → `rivet-store`
//! a forbidden edge. Naming the path here would be exactly the second authority
//! this module exists to eliminate. `PLAN.md` §10.7 records the same conclusion
//! for `apply edit`'s reserved table and the same remedy: **if it is wanted, the
//! way to get it is to move the constant**, not to copy the string.
//!
//! **`/state/app/<name>/`'s contents.** [`crate::service::APP_ROOT`] is a real
//! path and is declared below; `current`, `bin`, `manifest.toml` and `data`
//! ([`crate::service::DATA_DIR`]) are *names inside a directory whose middle
//! component is an application's*, which is a pattern rather than a path.
//! `explain` takes a path and answers about it; a pattern needs a matcher, and
//! a matcher is a bigger operation than this one is. The entry for `APP_ROOT`
//! says what is under it so the answer is not silent about it.
//!
//! **Anything on `/system`.** `ADR-018` §3 makes it read-only and an update
//! replaces the whole slot, so no path on it is *generated* in this sense — it
//! is *shipped*, by `cmd/rivet-image`, and the question "what would I change"
//! has one answer for all of it: build another image. That answer belongs in
//! one place rather than repeated per path, and it is not this table.
//!
//! # This module reads no filesystem
//!
//! `ADR-014` §3.1's standing check. Every field below is a property of Rivet
//! rather than of any particular boot, which is what makes the question
//! answerable from a workstation **before** the mistake — `ADR-007` §3.6.1
//! item 3's whole reason for existing. The consequence is the same one
//! `crate::operation::explain` already states: this says what generates a path,
//! never whether the file is there.

use crate::deploy;
use crate::journal;
use crate::service;
use crate::state;
use crate::uninstall_library;

/// What kind of thing a generated path is.
///
/// **Closed**, per `ADR-006` §6.4's closed-vocabulary rule. Each variant names
/// a distinct *relationship between the operator and the bytes*, because that
/// is the thing an operator is actually asking about when they run `explain`,
/// and it is what decides whether editing the file is futile, harmful or fine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Kind {
    /// A read-only render of state owned elsewhere, replaced wholesale every
    /// time the machine renders it. `ADR-007` §3.6.2. An edit is **discarded**,
    /// silently, at the next render — which for most of these is the next boot.
    Projection,
    /// A directory Rivet creates so that something else can work. Its mode is
    /// load-bearing; its contents are somebody else's.
    Directory,
    /// State the machine generated **once** and now owns. Not derived from
    /// anything, so there is nothing to change it *from*; changing it changes
    /// the machine's identity.
    MachineState,
    /// An append-only account of what happened. `ADR-022`. One writer, and it
    /// is not the operator.
    Record,
    /// A channel. A file or socket whose whole purpose is that a process writes
    /// it and another process reads it — so writing it by hand does not
    /// *corrupt* anything, it **actuates** something, which is worse.
    Mechanism,
}

impl Kind {
    /// The word `explain` puts in its `kind` field.
    pub const fn word(self) -> &'static str {
        match self {
            Self::Projection => "projection",
            Self::Directory => "directory",
            Self::MachineState => "machine state",
            Self::Record => "record",
            Self::Mechanism => "mechanism",
        }
    }
}

/// One path Rivet generates outside `/etc`.
///
/// Shaped after [`crate::etc::EtcDefinition`] on purpose — an operator reading
/// two answers from one verb should not be able to tell which table they came
/// from — with three differences, each of which is a fact the `/etc` table can
/// hold constant and this one cannot:
///
/// * **`kind`**, because `/etc` is entirely [`Kind::Projection`] and this is not.
/// * **`filesystem`**, because `/etc` is one tmpfs and these are spread across
///   `/state` and `/run`.
/// * **`mode` is optional**, because `rivet-core` does not own every one of
///   them. Where it does not, saying so is the honest answer and inventing a
///   number would be a second authority for a fact another crate holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeneratedDefinition {
    /// Absolute, and where it lands on the booted machine.
    pub path: &'static str,
    /// The operator's relationship to the bytes.
    pub kind: Kind,
    /// What puts it there — a program, named the way `ps` names it.
    pub generated_by: &'static str,
    /// What the contents are computed from, in the same voice
    /// [`crate::etc::EtcDefinition::source`] uses.
    ///
    /// [`Kind::MachineState`] and [`Kind::Directory`] entries are derived from
    /// nothing, and say so in words rather than by being empty.
    pub derived_from: &'static str,
    /// Whether an operator may write it and expect the write to mean something.
    ///
    /// **This is the field the module exists for.** `false` here does not mean
    /// the kernel will refuse the write — `/state` is writable and root is root.
    /// It means the write will not survive, or will not do what it looks like it
    /// does. [`Self::why`] is where that distinction is spelled out, every time.
    pub writable: bool,
    /// What it lives on, and why that matters to a write.
    pub filesystem: &'static str,
    /// The mode Rivet creates it with, where `rivet-core` is the authority
    /// for that number. `None` where it is not.
    pub mode: Option<u32>,
    /// Why Rivet generates it at all, in one sentence.
    pub why: &'static str,
    /// What an operator changes in order to change it.
    ///
    /// **Never "edit it"** for anything whose [`Self::writable`] is `false` —
    /// `ADR-007` §3.6.2 rules that a generated file is a view and never an
    /// interface. Where there is genuinely no mechanism, this says so plainly
    /// rather than naming one that does not exist.
    pub change_with: &'static str,
}

/// Every path Rivet generates outside `/etc` that this crate is authoritative
/// for, **parent before child**.
///
/// **Parent before child**, for the reason a reader feels immediately: a reader
/// meeting `/state/log/journal.jsonl` should already have met `/state/log`, and
/// `explain`'s refusal lists them in this order. `a_parent_never_follows_its_child`
/// is what keeps it true.
///
/// **Depth order was tried first and was wrong.** It is easy to test and it
/// splits `/state/log` from `/state/log/journal.jsonl` to put
/// `/state/deploy-request` between them, which makes the list harder to read
/// than the prose it replaces. Parent-before-child is the property that actually
/// matters and it permits the grouping.
/// **What version of Rivet this machine is running** — `/run/rivet/os-version`.
///
/// Published by the supervisor, which ships **in the image**, so its own crate
/// version *is* the image's. Read by anything that wants to say what OS it is
/// on without assuming it was built alongside it.
///
/// # The bug this exists for, which the code that had it predicted
///
/// `cmd/rivet-launcher` drew `Rivet OS {CARGO_PKG_VERSION}` — its OWN version —
/// and its comment said, before anyone noticed:
///
/// > *A launcher deployed from a different tree than the image it runs on would
/// > say so wrongly, and that is a thing to fix by asking the machine rather
/// > than by hedging the string.*
///
/// Which is exactly what happened. The launcher is deployed as an APPLICATION
/// while it is iterated on (`.launcher-pkg`, `PLAN.md` §104.36), so its version
/// is whenever it was last installed — and `rivet-01` spent days drawing
/// `Rivet OS 0.1.61` on a 0.1.63 image, with `/etc/motd` on the same machine
/// saying `v0.1.63`.
///
/// **Not in `/etc`**, for `crate::display::HOURS_PATH`'s reason: that surface
/// is for paths a C runtime demands, and no C runtime asks what version of
/// Rivet this is. A person reads this one, which is a different thing and
/// not a parser's input.
pub const OS_VERSION_PATH: &str = "/run/rivet/os-version";

pub const GENERATED_PATHS: &[GeneratedDefinition] = &[
    GeneratedDefinition {
        path: OS_VERSION_PATH,
        kind: Kind::Projection,
        generated_by: "the supervisor, once at start, from its own crate version - which is \
                       the IMAGE's, because the supervisor ships in the image",
        derived_from: "nothing configurable: it is what this machine was built as",
        writable: false,
        filesystem: "tmpfs at /run",
        mode: Some(0o644),
        why: "so a program can say what OS it is running on WITHOUT assuming it was built \
              alongside it. cmd/rivet-launcher assumed exactly that and drew Rivet OS 0.1.61 \
              on a 0.1.63 machine for days, because it is deployed as an application while it \
              is iterated on",
        change_with: "nothing. It is the version of the image, and an image is changed by \
                      installing a different one",
    },
    GeneratedDefinition {
        path: crate::backup::PATH,
        kind: Kind::Projection,
        generated_by: "the supervisor, from every installed application's SIGNED manifest, \
                       once per convergence (ADR-051)",
        derived_from: "the backup-path, backup-format, backup-feed and restore-path keys of \
                       each installed application's manifest, joined to the proxy-path it is \
                       mounted at. NOTHING here comes from the document",
        writable: false,
        filesystem: "tmpfs at /run",
        mode: Some(0o644),
        why: "ADR-030 rules application data OUT of Rivet's own backup - whole /state \
              `remains the operator's` - so an application that wants its data kept has to \
              OFFER it, and something outside has to be able to see the offer. This is the \
              offer, machine-readable, read with `get file` from a workstation. It is not \
              beside an application the way the proxy's routing table is, because it has no \
              reader on this machine at all",
        change_with: "nothing on this machine. A backup surface is inside the signature \
                      (ADR-015 §4), so changing one means building and signing a new .rvt \
                      and installing it",
    },
    GeneratedDefinition {
        path: crate::display::HOURS_PATH,
        kind: Kind::Projection,
        generated_by: "the supervisor, from the document's [display] table, once per \
                       convergence (ADR-039 §3.1)",
        derived_from: "the [display] table of the configuration document - `off` and `on`, \
                       two times of day in this machine's LOCAL time",
        writable: false,
        filesystem: "tmpfs at /run",
        mode: Some(0o644),
        why: "the machine owns WHEN its screen is lit and the DRM lease makes whichever \
              process holds the display the only thing that can act on it (ADR-039 §8.1), so \
              the policy has to reach that process - and an application cannot read the \
              configuration document. NOT in /etc, deliberately: that surface exists for C \
              runtimes that demand an exact path, and no C runtime asks for a display \
              schedule",
        change_with: "the [display] table of the configuration document - remove it and this \
                      file is removed too, which is a screen that is never switched off",
    },
    GeneratedDefinition {
        path: state::STATE_MOUNT_POINT,
        kind: Kind::Directory,
        generated_by: "PID 1's stage 0 mount plan, before the supervisor starts (ADR-017 §3.1.6)",
        derived_from: "nothing — it is a mount point shipped empty in the image, because the \
                       root filesystem it would otherwise be created on is read-only \
                       (ADR-018 §3)",
        writable: true,
        filesystem: "ext4 on the partition labelled state — the ONLY writable filesystem on \
                     a Rivet machine, and the only one an update does not replace \
                     (ADR-018 §4)",
        mode: None,
        why: "everything a machine learns after it was built lives here, so that an A/B \
              update can replace all of /system and lose none of it",
        change_with: "nothing: the partition table is fixed when the card is written \
                      (ADR-018 §4). Where files under it come from is each entry below",
    },
    GeneratedDefinition {
        path: service::APP_ROOT,
        kind: Kind::Directory,
        generated_by: "the supervisor, when it first discovers an installed application \
                       (ADR-015 §4.2)",
        derived_from: "nothing — it is the parent of one directory per installed application",
        writable: false,
        filesystem: "ext4 at /state",
        mode: None,
        why: "an application's versions, its program and its private data all live under \
              one name, so that an install adds a directory and an uninstall removes one",
        change_with: "rivet apply install <DIGEST> confirm. Under it, each application \
                      has a version tree, a bin directory, a manifest and a data directory \
                      owned by that application's own uid (ADR-015 §5.1.1) — none of which \
                      this operation can answer about individually, because the middle \
                      component is an application's name rather than a fixed path",
    },
    GeneratedDefinition {
        path: state::CONFIG_DIR,
        kind: Kind::Directory,
        generated_by: "whatever first persists a layer of desired state (ADR-017 §3.1.7)",
        derived_from: "nothing — it is a directory holding one file per persisted layer of \
                       ADR-007 §3.4's desired state, and rivet-store owns their names",
        writable: false,
        filesystem: "ext4 at /state",
        mode: None,
        why: "ADR-008 §3.1 puts Rivet's own configuration under a name that states both \
              ownership and lifetime, which /config would state neither of",
        change_with: "the provisioning document on the boot partition on a first boot \
                      (ADR-004 §6). This build has no verb that writes a layer directly — \
                      ADR-006 §6.4's `set` does not exist",
    },
    GeneratedDefinition {
        path: state::ARTIFACTS,
        kind: Kind::Directory,
        generated_by: "the control plane's bulk endpoint, when an artifact is transferred to \
                       this machine (ADR-052 §7)",
        derived_from: "nothing — it is a digest-keyed object store holding one directory per \
                       artifact, named by that artifact's BLAKE3 digest, each holding one \
                       ADR-027-named .rvt",
        writable: false,
        filesystem: "ext4 at /state",
        mode: None,
        why: "an install names a digest and resolves it here, and there is no form of it \
              that names a path — so nothing reaches this machine's installer that did not \
              arrive through a transfer that hashed it (ADR-052)",
        change_with: "`cargo rivet deploy`, which transfers an artifact into this store and \
                      then asks for the install by the digest it sent. A .rvt PLACED HERE BY \
                      HAND under a directory named by its own digest would be installable — \
                      the boundary is root on /state and always was — but nothing prunes it \
                      correctly unless its name is the one ADR-027 §7 composes, and RETENTION \
                      DELETES FROM THIS DIRECTORY: the three most recently added artifacts of \
                      each product are kept and the rest are removed when the next one \
                      arrives. Do not keep anything here that is not a copy",
    },
    GeneratedDefinition {
        path: state::LOG_DIR,
        kind: Kind::Directory,
        generated_by: "the supervisor, which is the journal (ADR-022 §7)",
        derived_from: "nothing — it holds the active file and the rotated ones",
        writable: false,
        filesystem: "ext4 at /state",
        mode: Some(state::LOG_DIR_MODE),
        why: "ADR-022 §8 keeps the journal in size-capped files with a capped total, \
              because an unbounded log kills the card (ADR-018 §7)",
        change_with: "nothing. Read it with `rivet get log`",
    },
    GeneratedDefinition {
        path: state::ACTIVE_LOG,
        kind: Kind::Record,
        generated_by: "the supervisor, and it is the ONLY writer (ADR-022 §7)",
        derived_from: "the records every Rivet process files — its own directly, and \
                       everybody else's through the ingest socket",
        writable: false,
        filesystem: "ext4 at /state",
        mode: Some(state::LOG_FILE_MODE),
        why: "one process holds the file open so that two appenders can never interleave a \
              partial line, which would damage somebody else's record rather than the \
              writer's own (ADR-022 §11)",
        change_with: "nothing writes it by hand. To FILE a record, send it to the ingest \
                      socket; to READ it, `rivet get log`. An append by hand races the \
                      rotation this build performs and is the exact hazard the socket \
                      exists to remove",
    },
    GeneratedDefinition {
        path: deploy::REQUEST_PATH,
        kind: Kind::Mechanism,
        generated_by: "`rivet apply install`, which writes it; the supervisor, which \
                       adopts it and creates the installer's service instance",
        derived_from: "the artifact DIGEST, resolved in this machine's own store, and the authenticated principal of the control-plane \
                       session that asked (ADR-028 §4.5)",
        writable: false,
        filesystem: "ext4 at /state",
        mode: Some(deploy::REQUEST_MODE),
        why: "an update writes hundreds of megabytes, and a client that owned that write \
              directly would own a half-written slot the moment its connection dropped \
              (the maintainer's 2026-08-24 ruling)",
        change_with: "rivet apply install <DIGEST> confirm. Writing this file by hand \
                      does not describe an install, it REQUESTS one — the supervisor acts \
                      on whatever it finds here",
    },
    GeneratedDefinition {
        path: deploy::REQUEST_TEMP_PATH,
        kind: Kind::Mechanism,
        generated_by: "`rivet apply install`, as the temporary half of a \
                       write-sync-rename",
        derived_from: "the same request, before it is atomically renamed into place",
        writable: false,
        filesystem: "ext4 at /state",
        mode: Some(deploy::REQUEST_MODE),
        why: "a request is either wholly there or wholly absent, so the supervisor can \
              never adopt a half-written one",
        change_with: "nothing. Finding one left behind means an install request died \
                      between the write and the rename, and it is the only evidence of it",
    },
    GeneratedDefinition {
        path: uninstall_library::REQUEST_PATH,
        kind: Kind::Mechanism,
        generated_by: "`rivet apply uninstall-library`, which writes it; the supervisor, which \
                       reads it once per convergence pass and removes it before acting on it",
        derived_from: "the library selector the operator typed — a product name or a full \
                       reference — and the authenticated principal of the control-plane \
                       session that asked (ADR-028 §4)",
        writable: false,
        filesystem: "ext4 at /state",
        mode: Some(uninstall_library::REQUEST_MODE),
        why: "only the supervisor can recompose the library set every service is given \
              without a reboot, and it reads every installed application's declared closure \
              again before it removes a build, refusing the whole request if one names it",
        change_with: "rivet apply uninstall-library <product|reference> confirm. Writing this \
                      file by hand does not describe a removal, it REQUESTS one — the \
                      supervisor acts on whatever it finds here",
    },
    GeneratedDefinition {
        path: uninstall_library::REQUEST_TEMP_PATH,
        kind: Kind::Mechanism,
        generated_by: "`rivet apply uninstall-library`, as the temporary half of a \
                       write-sync-rename",
        derived_from: "the same request, before it is atomically renamed into place",
        writable: false,
        filesystem: "ext4 at /state",
        mode: Some(uninstall_library::REQUEST_MODE),
        why: "a request is either wholly there or wholly absent, so the supervisor can never \
              act on a half-written one",
        change_with: "nothing. Finding one left behind means a library uninstall request died \
                      between the write and the rename, and it is the only evidence of it",
    },
    GeneratedDefinition {
        path: journal::INGEST_SOCKET_PATH,
        kind: Kind::Mechanism,
        generated_by: "the supervisor, which binds it and is the only reader (ADR-022 §11)",
        derived_from: "nothing — it is a socket, not a file with contents",
        writable: false,
        filesystem: "tmpfs at /run, so it exists only while this boot does",
        mode: Some(journal::INGEST_SOCKET_MODE),
        why: "a process the supervisor did not spawn still has records to file, and the \
              alternative — every writer appending to the journal itself — lets two \
              appenders interleave a partial line (ADR-022 §11, PLAN.md §11.1)",
        change_with: "nothing. SEND to it: the CLI and femto already do, and the receiver \
                      stamps the sender's uid itself, so a record's origin cannot be \
                      forged by what is written into it",
    },
];

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::etc::ETC_FILES;

    /// Every path is absolute. `explain` takes an absolute path and nothing
    /// else, so a relative entry here would be unreachable rather than wrong.
    #[test]
    fn every_path_is_absolute() {
        for def in GENERATED_PATHS {
            assert!(
                def.path.starts_with('/'),
                "{} is not absolute",
                def.path
            );
        }
    }


    /// No path appears twice. Two entries for one path means `explain` answers
    /// with whichever came first, which is the silent-wrong-answer failure this
    /// module exists to remove.
    #[test]
    fn no_path_is_declared_twice() {
        let mut seen: Vec<&str> = GENERATED_PATHS.iter().map(|d| d.path).collect();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(before, seen.len(), "a path is declared twice: {seen:?}");
    }

    /// **The three tables are disjoint.** `ADR-011` §2 — one thing owns each
    /// fact. A path in two of them has two owners, and `explain` would have to
    /// choose.
    #[test]
    fn the_tables_do_not_overlap() {
        for def in GENERATED_PATHS {
            assert!(
                !ETC_FILES.iter().any(|e| e.path == def.path),
                "{} is in ETC_FILES too",
                def.path
            );
        }
    }

    /// Nothing on `/etc` belongs in this table — that is `ETC_FILES`' half of
    /// the split, and an entry here would be the second spelling.
    #[test]
    fn nothing_here_is_under_etc() {
        for def in GENERATED_PATHS {
            assert!(
                !def.path.starts_with("/etc"),
                "{} belongs to the /etc projection",
                def.path
            );
        }
    }

    /// Parent before child, so the refusal reads as a walk down the tree rather
    /// than as the order things were added.
    ///
    /// A path is a parent of another when the other starts with it **followed by
    /// a separator** — the trailing slash is the whole test, because without it
    /// `/state/deploy-request` is a "parent" of `/state/deploy-request.new` by
    /// prefix and is not one on the filesystem.
    #[test]
    fn a_parent_never_follows_its_child() {
        for (later, def) in GENERATED_PATHS.iter().enumerate() {
            for other in GENERATED_PATHS.iter().skip(later + 1) {
                assert!(
                    !def.path.starts_with(&format!("{}/", other.path)),
                    "{} comes before its parent {}",
                    def.path,
                    other.path
                );
            }
        }
    }

    /// Every sentence is a sentence. An empty field renders as an empty answer,
    /// which is the shrug `AGENTS.md` §5 refuses — an answer must name
    /// something, even when the something is "nothing".
    #[test]
    fn no_field_is_empty() {
        for def in GENERATED_PATHS {
            for (name, text) in [
                ("generated_by", def.generated_by),
                ("derived_from", def.derived_from),
                ("filesystem", def.filesystem),
                ("why", def.why),
                ("change_with", def.change_with),
            ] {
                assert!(!text.trim().is_empty(), "{}'s {name} is empty", def.path);
            }
        }
    }

    /// **`change_with` never says "edit it"** for anything not writable.
    /// `ADR-007` §3.6.2: a generated file is a view and never an interface, and
    /// the field that tells an operator what to do is the one place that rule
    /// can be broken in a way nobody notices.
    #[test]
    fn a_view_is_never_offered_as_an_interface() {
        for def in GENERATED_PATHS.iter().filter(|d| !d.writable) {
            let advice = def.change_with.to_ascii_lowercase();
            for forbidden in ["edit it", "edit this", "edit the file"] {
                assert!(
                    !advice.contains(forbidden),
                    "{} is not writable and its advice says {forbidden:?}",
                    def.path
                );
            }
        }
    }

    /// The mode, where a mode is claimed, is one this crate really owns —
    /// checked by identity against the constant rather than by value, so a
    /// change to the constant moves this table with it instead of leaving a
    /// literal behind.
    #[test]
    fn the_modes_are_the_constants_and_not_copies_of_them() {
        let mode = |path: &str| {
            GENERATED_PATHS
                .iter()
                .find(|d| d.path == path)
                .unwrap_or_else(|| panic!("{path} is not in the table"))
                .mode
        };
        assert_eq!(mode(state::LOG_DIR), Some(state::LOG_DIR_MODE));
        assert_eq!(mode(state::ACTIVE_LOG), Some(state::LOG_FILE_MODE));
        assert_eq!(mode(deploy::REQUEST_PATH), Some(deploy::REQUEST_MODE));
        assert_eq!(
            mode(uninstall_library::REQUEST_PATH),
            Some(uninstall_library::REQUEST_MODE)
        );
        assert_eq!(
            mode(journal::INGEST_SOCKET_PATH),
            Some(journal::INGEST_SOCKET_MODE)
        );
    }

    /// **The rule this module was written for.**
    ///
    /// A projection is a view: it is not writable, and its `change_with` names
    /// what an operator changes instead. A projection that told an operator to
    /// edit it is the failure this whole table exists to prevent — an edit that
    /// works until the next boot and then silently is not there.
    #[test]
    fn a_projection_is_never_offered_as_an_interface() {
        for def in GENERATED_PATHS {
            if def.kind != Kind::Projection {
                continue;
            }
            assert!(!def.writable, "{} is a projection and writable", def.path);
            assert!(
                !def.change_with.to_lowercase().contains("edit it"),
                "{} offers itself as an interface (ADR-007 §3.6.2)",
                def.path
            );
        }
    }

    /// Every [`Kind`] renders a distinct word, because the word is what an agent
    /// matches on.
    #[test]
    fn the_kinds_have_distinct_words() {
        let mut words = [
            Kind::Projection,
            Kind::Directory,
            Kind::MachineState,
            Kind::Record,
            Kind::Mechanism,
        ]
        .map(Kind::word)
        .to_vec();
        words.sort_unstable();
        let before = words.len();
        words.dedup();
        assert_eq!(before, words.len(), "two kinds render the same word");
    }
}
