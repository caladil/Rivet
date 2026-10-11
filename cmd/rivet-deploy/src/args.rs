//! What `argv` asked for.
//!
//! # This is the part that is **not ruled**, and it is said here rather than
//! discovered
//!
//! The maintainer's 2026-08-24 ruling made the installer its own supervised binary and
//! left two things open in the same paragraph:
//!
//! > What follows and is not yet ruled: whether it is one-shot or long-running,
//! > and how an operator *asks* it to install something — `ADR-017` §3.4's
//! > notify socket is the obvious channel and nothing has decided.
//!
//! **Both are answered here provisionally, in the smallest form that works, and
//! the maintainer's 2026-08-24 install-request ruling carries the alternatives and asks for a ruling**
//! (`AGENTS.md` §1 — a decision that changes the shape of the software is the
//! maintainer's, and this file must not be where it is quietly taken).
//!
//! * **One-shot.** The binary installs one payload and exits, on
//!   `cmd/rivet-netd`'s precedent: `ADR-017` §4.3 ruled that *"run-once needs
//!   no new vocabulary"* — `on-failure` plus the supervisor's `Done` state is
//!   exactly this shape, and it brings the backoff, the give-up threshold and
//!   the SD-card protection of §3.5 with it.
//! * **The payload's path on `argv`.** Not a notify socket: `ADR-017` §3.4's
//!   `NOTIFY_SOCKET` is `sd_notify`'s convention and runs *service → supervisor*,
//!   which is the wrong direction for an instruction. A path is what a one-shot
//!   takes.
//!
//! **What that leaves open is the thing to flag, not the thing to hide.** A
//! supervised service's argv is `cmd/rivet-sysd`'s
//! `definitions::system_services()`, a compile-time constant that carries no
//! parameter — so *today* there is no way for an operator to start this binary
//! with a path through the supervisor at all, and running it from the login
//! shell is the session-owns-the-write hazard ruling 1 exists to remove. The binary
//! is the half that can be built and tested now; the channel is a ruling.

use crate::kernel::ConsoleForm;

/// What the command line asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Usage,
    Install(Options),
}

/// An install, as asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// The `.rvt` to install.
    pub payload: String,
    /// Keys to trust **instead of** the machine's own trust store — an
    /// override, and empty in the ordinary case.
    ///
    /// ~~A path rather than a location, and that is a gap being named rather
    /// than a design.~~ **Struck 2026-08-24 — `ADR-028` 5.8.** Something puts a
    /// trust store in `/system` now: `cmd/rivet-image` writes
    /// `trust_store::PATH`, so this binary reads the machine's own answer by
    /// default and no longer has to be told one.
    ///
    /// The flag survives because two cases still need it — a card built
    /// `--no-trust`, and verifying a payload against a key that is not on this
    /// machine. Both are deliberate acts, so an override is journalled at
    /// `Warn` and narrowed to `update` alone: a key an operator hands to this
    /// binary is entitled to authorise exactly the thing this binary does.
    pub trusted_keys: Vec<String>,
    /// Verify everything, plan everything, **write nothing**. `ADR-006` §6.2:
    /// *"every mutating operation supports a dry run."*
    pub dry_run: bool,
    /// `ADR-029` §9.2 step 9 — read the written system partition back and hash
    /// it. **On by default**, because the ADR recommends it and because it is
    /// the only defence against a card that accepts a write and stores
    /// something else.
    ///
    /// The flag turns it off, and the cost of doing so is stated in the usage
    /// text rather than left to a reader of this field.
    pub read_back: bool,
    /// How records are rendered. `ADR-022` §3's JSONL by default, because
    /// `ADR-022` §4's capture parses that form back into records.
    pub form: ConsoleForm,
}

/// `argv`, minus the program name.
pub fn parse(argv: &[String]) -> Result<Request, String> {
    let mut payload: Option<String> = None;
    let mut trusted_keys: Vec<String> = Vec::new();
    let mut dry_run = false;
    let mut read_back = true;
    let mut form = ConsoleForm::Jsonl;

    let mut rest = argv.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(Request::Usage),
            "--trusted-key" => {
                let path = rest
                    .next()
                    .ok_or_else(|| "--trusted-key needs a path".to_string())?;
                trusted_keys.push(path.clone());
            }
            "--dry-run" => dry_run = true,
            "--no-read-back" => read_back = false,
            "--text" => form = ConsoleForm::Text,
            other if other.starts_with('-') => {
                return Err(format!("unknown option {other}"));
            }
            other => {
                if payload.is_some() {
                    return Err(format!(
                        "one payload at a time; {other} is the second. An install writes \
                         640 MiB into one slot, and there is only one inactive slot"
                    ));
                }
                payload = Some(other.to_string());
            }
        }
    }

    let payload = payload.ok_or_else(|| "no payload named".to_string())?;
    // No `--trusted-key` is now the ORDINARY case: the machine's own trust
    // store answers, and `install` refuses if it has none. Refusing here would
    // refuse the common path in order to guard a case that has its own,
    // better-informed refusal — one that can say whether the store is missing,
    // unreadable or empty (ADR-028 5.8).
    Ok(Request::Install(Options {
        payload,
        trusted_keys,
        dry_run,
        read_back,
        form,
    }))
}

/// The usage text. One place, so `--help` and a refusal cannot disagree.
pub fn usage() -> String {
    format!(
        "\
{name} — install a signed .rvt into this machine's inactive slot. ADR-029 §9.2.

usage: {name} [options] <payload.rvt>

  --trusted-key PATH   a 32-byte Ed25519 public key, raw or hex, to accept
                       INSTEAD of this machine's own trust store. Repeatable,
                       and NOT required: by default the store the image
                       carries at /system/trust answers, which is what
                       ADR-028 5.5.6 means by a machine saying what it trusts.
                       An override is for a card built --no-trust, or for a
                       key that is not on this machine. It is journalled at
                       warn, and it is narrowed to `update` alone.
  --dry-run            verify the payload and plan the install, writing
                       nothing. Reaches the same verdict the real run would,
                       so a refusal is discovered before a confirmation
                       rather than after it (ADR-006 §6.2).
  --no-read-back       do not re-read the written system partition and hash
                       it. ADR-029 §9.2 step 9 recommends the read-back and
                       says why: SD cards do accept writes and store something
                       else, and this is the only defence against that. It
                       costs one 640 MiB read.
  --text               render records as text rather than as ADR-022 §3's
                       JSONL. The JSONL form is what the supervisor's capture
                       parses back into records (ADR-022 §4), so it is the
                       default and --text is for a human at a terminal.
  --help

It installs and arms; it does NOT reboot. ADR-029 §9.1 — an armed machine is
inert until a trial reboot, so twenty machines can be installed at noon and
rebooted at 2am.
",
        name = crate::SELF_NAME
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn a_payload_and_a_key_are_enough() {
        let request = parse(&argv(&["--trusted-key", "/k", "/p.rvt"])).expect("parsed");
        let Request::Install(options) = request else {
            panic!("not an install");
        };
        assert_eq!(options.payload, "/p.rvt");
        assert_eq!(options.trusted_keys, vec!["/k".to_string()]);
        assert!(!options.dry_run);
        assert!(options.read_back, "the read-back must default to on");
        assert_eq!(options.form, ConsoleForm::Jsonl);
    }

    /// **No key on the command line is now the ordinary case**, because the
    /// machine's own store answers.
    ///
    /// The refusal this replaces was the most important one in this file, and
    /// what made it so is unchanged: a verifier with an empty trust set refuses
    /// every payload, which looks like a broken installer, and a verifier that
    /// chose its own key would be `ADR-028` 5.1's loop. Neither became
    /// acceptable — the refusal MOVED, to `install`, which reads the store and
    /// can therefore say whether it is missing, unreadable or empty rather than
    /// only that nobody named a path.
    #[test]
    fn an_install_with_no_trusted_key_parses_and_leaves_the_store_to_answer() {
        let request = parse(&argv(&["/p.rvt"])).expect("parsed");
        let Request::Install(options) = request else {
            panic!("not an install");
        };
        assert!(options.trusted_keys.is_empty());
        assert_eq!(options.payload, "/p.rvt");
    }

    /// And the usage text says where the answer comes from, so a reader who
    /// looked for a required flag and did not find one is told why.
    #[test]
    fn the_usage_names_the_store_the_machine_carries() {
        let text = usage();
        assert!(text.contains("/system/trust"), "{text}");
        assert!(text.contains("NOT required"), "{text}");
    }

    #[test]
    fn two_payloads_are_refused_rather_than_the_second_winning() {
        let why = parse(&argv(&["--trusted-key", "/k", "/a.rvt", "/b.rvt"])).expect_err("refused");
        assert!(why.contains("/b.rvt"), "{why}");
    }

    #[test]
    fn every_flag_is_read() {
        let request = parse(&argv(&[
            "--dry-run",
            "--no-read-back",
            "--text",
            "--trusted-key",
            "/k1",
            "--trusted-key",
            "/k2",
            "/p.rvt",
        ]))
        .expect("parsed");
        let Request::Install(options) = request else {
            panic!("not an install");
        };
        assert!(options.dry_run);
        assert!(!options.read_back);
        assert_eq!(options.form, ConsoleForm::Text);
        assert_eq!(options.trusted_keys.len(), 2);
    }

    #[test]
    fn an_unknown_option_is_refused_rather_than_taken_as_a_payload_name() {
        let why = parse(&argv(&["--trusted-key", "/k", "--wat", "/p.rvt"])).expect_err("refused");
        assert!(why.contains("--wat"), "{why}");
    }

    #[test]
    fn a_trailing_option_with_no_value_is_refused() {
        assert!(parse(&argv(&["/p.rvt", "--trusted-key"])).is_err());
    }

    #[test]
    fn help_needs_nothing_else_and_wins() {
        assert_eq!(parse(&argv(&["--help"])), Ok(Request::Usage));
        assert_eq!(
            parse(&argv(&["--trusted-key", "/k", "/p.rvt", "-h"])),
            Ok(Request::Usage)
        );
    }

    /// The usage text is the only place the flags are described, so a flag that
    /// exists and is undocumented is a defect a test can catch.
    #[test]
    fn every_flag_the_parser_accepts_appears_in_the_usage_text() {
        let text = usage();
        for flag in [
            "--trusted-key",
            "--dry-run",
            "--no-read-back",
            "--text",
            "--help",
        ] {
            assert!(text.contains(flag), "{flag} is undocumented");
        }
        assert!(
            text.contains("does NOT reboot"),
            "the usage text must say the arming is not a reboot — ADR-029 §9.1"
        );
    }
}
