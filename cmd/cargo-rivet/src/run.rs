//! `cargo rivet run` — **one operation on a machine, over its own control
//! plane, in its own words.**
//!
//! The gap this closes was measured rather than argued. Before it, `cargo rivet`
//! offered `package`, `update`, `deploy`, `inspect`, `key` and `identity`, and
//! **only `deploy --identity` spoke `ADR-035`'s control plane at all**. To ask a
//! board anything else from a workstation an operator opened a login session or
//! hand-wrote JSON-RPC — so the login was the path of least resistance because
//! it was the only path that worked for a general query. `ADR-035`'s phase 5
//! leaves the control plane alone at the far end; one that cannot carry `get
//! storage` is not yet the interface, whatever the README says.
//!
//! # This module is a command line and nothing else
//!
//! The translation lives in `rivet_remote::command`, for `ADR-034` §11.5's
//! reason: `ADR-014` makes `cmd/*` a composition root, and *"a composition root
//! is not where an implementation two tools share belongs"* — `rivet-fleet` is
//! the second tool, and it will want this the day it wants to ask a device a
//! question it has no verb for.
//!
//! # What this has been run against, 2026-08-29
//!
//! `AGENTS.md` §6. **Not a board** — none was available.
//!
//! What it has done is carry a command end to end on one host: a Windows build
//! of `cmd/rivet-mcpd` bound to `127.0.0.1:7423` with a minted identity and a
//! peers file, and
//!
//! ```text
//! cargo rivet run --host 127.0.0.1:7423 --identity … --pin … -- explain /etc/passwd
//! ```
//!
//! printed the operation's own JSON and exited `0`, with the daemon reporting
//! *"TLS 1.3, mutually authenticated"* and `rivet mcp` exiting `0`. `get log
//! previous` answered; `get storage`, `apply reload` and `get stroage` came back
//! as the machine's own refusals with a non-zero status; a wrong `--pin` was
//! refused by the trust model before anything was sent.
//!
//! **7423 above is what that run used and is not this project's port.** It is
//! left here because it is a record of what was done (`AGENTS.md` §5) rather
//! than an example to copy — and because that same number was, the same day,
//! one of **three** control-plane ports a cold agent found in three places
//! (`PLAN.md` §92, defect 4). The one to copy is
//! `rivet_config::control_plane::DEFAULT_PORT`, which is 7777 and which this
//! module's usage now uses throughout —
//! `every_port_in_the_help_is_the_control_planes_default` is what keeps it so.
//!
//! **What that does not establish** is anything about a Raspberry Pi. Every
//! operation needing a running Linux refused for that reason, so no `get
//! storage` has yet returned a filesystem. That closes against hardware and
//! nothing else.
//!
//! # There is one transport, and that is the point rather than an omission
//!
//! `--identity` and `--pin` are REQUIRED, here and in `deploy`. There is one
//! channel to a machine and its trust is mutual (`ADR-025` §2), so a run with
//! neither flag is a run with nothing to present and nothing to pin. A fallback
//! would defeat the whole purpose, and there is nothing left to fall back to.

use rivet_remote::endpoint::{Endpoint, IDENTITY_AND_PIN};
// The narrow public surface of `rivet_remote::tls` — one tool call, and the
// text it answered with. Everything under it stays that module's.
use rivet_remote::tls::CallsTools as _;

pub fn usage() -> String {
    format!(
        "usage: cargo rivet run <device> -- <rivet command>\n\
         \x20      cargo rivet run --host HOST:PORT --identity DIR --pin HEX -- <rivet command>\n\
         \n\
         Runs ONE operation from the machine's own catalogue over ADR-035's\n\
         control plane and prints the machine's answer, unchanged.\n\
         \n\
         \x20 cargo rivet run rivet-01 -- get storage\n\
         \x20 cargo rivet run --host pi:7777 --identity ./id --pin <hex> -- get storage\n\
         \x20 cargo rivet run --host pi:7777 --identity ./id --pin <hex> -- apply reload confirm\n\
         \n\
         A BARE LEADING WORD before `--` is a device name (PLAN-Feature-Set-2026-09-12\n\
         §7.1), resolved against rivet-fleet's own register into --host, --identity\n\
         and --pin: whichever of the three you also write explicitly is kept, and the\n\
         register only fills in what you did not write. A name the register does not\n\
         hold is refused, naming every name it does. A record with no address - a\n\
         DHCP machine - is dialled at whatever <name>.local resolves to through this\n\
         workstation's resolver (IPv4 first, IPv6 only when nothing else answers),\n\
         on the record's port, and one line names the address used. The register's\n\
         pin still decides whether to trust what answers there. If the name does not\n\
         resolve, this refuses; --host HOST:PORT names the machine directly.\n\
         Nothing on the register decides which identity to use, so this reads it\n\
         from the standing convention ~/.rivet/identity/operator (README.md, ADR-019).\n\
         \n\
         run options:\n\
         \x20 --host HOST:PORT   The address and port rivet-mcpd is listening on. The\n\
         \x20                    port is not guessed. REQUIRED unless a device name\n\
         \x20                    supplied one.\n\
         {IDENTITY_AND_PIN}\
         \x20 --show             print the tool call this would send and send\n\
         \x20                    nothing. Needs no identity, no pin and no machine\n\
         \x20                    — a translation does not depend on which board it\n\
         \x20                    is for — so it is how to see the exact arguments\n\
         \x20                    object before a command touches anything.\n\
         \x20 --full             do not cap the printed answer. Without it, an\n\
         \x20                    answer over {CAP_BYTES} bytes is cut and a notice\n\
         \x20                    names the real size — `get log latest all` against\n\
         \x20                    a busy board answers several megabytes, which is\n\
         \x20                    enough to exhaust an agent's whole context window\n\
         \x20                    on one command. This is this TOOL's cap, separate\n\
         \x20                    from `get log`'s own `all`/default limit on the\n\
         \x20                    machine (ADR-022 §9.6) — `--full` bypasses this\n\
         \x20                    one; the words after `--` are how you ask the\n\
         \x20                    MACHINE for less. Output redirected to a\n\
         \x20                    regular file is never cut: nothing reads it as\n\
         \x20                    it is written, and a cut would leave JSON that\n\
         \x20                    does not parse.\n\
         \x20 --                 REQUIRED, and everything after it is the command.\n\
         \n\
         SOME IDENTITY AND SOME PIN ARE ALWAYS REQUIRED, because this verb speaks the\n\
         control plane and there is nothing else — either written out as --identity\n\
         and --pin, or supplied by a device name's record. There is deliberately no\n\
         fallback beyond the register: a channel that quietly used a key it was asked\n\
         not to use would be the worst of both.\n\
         \n\
         THE WORDS AFTER `--` ARE A RIVET COMMAND, not a shell command line and\n\
         not this tool's flags. They are bound to the operation's DECLARED inputs\n\
         by position, from the catalogue — so a flag `rivet` itself consumes,\n\
         `--follow` above all, is not an input of any operation and cannot be\n\
         sent this way. `rivet --version` on the machine publishes the catalogue,\n\
         which is generated from the types and is the authority for what exists.\n\
         \n\
         The answer is printed exactly as the machine composed it, up to the cap\n\
         above: this tool adds nothing and summarises nothing beyond that one\n\
         notice. A refusal is the MACHINE's refusal, on stderr, and the exit\n\
         status is non-zero for it.\n\
         \n\
         A command this end cannot translate is refused before anything is sent —\n\
         a word that is not one of ADR-006 §6.4's verbs, a verb this build has no\n\
         operation behind, or a word with no declared position to hold it.\n"
    )
}

/// **`PLAN-Feature-Set-2026-09-12` §7.2's first agent-experience principle,
/// applied to this tool's own output**: truncation with an escape hatch.
///
/// # Why a cap belongs HERE and not only on the machine
///
/// `rivet-core`'s `get log` already caps itself at 200 records by default and
/// says so (`ADR-022` §9.6) — but its escape hatch, `all`, is exactly the word
/// that produces the answer this cap exists for: `get log latest all` against
/// a board that has been running measured **four megabytes**, and this tool
/// used to print every byte of it unchanged. An agent asking for `all`
/// because 200 records were not enough would blow its own context window on
/// the answer to that second, more specific question. The two caps are not
/// the same knob: the machine's decides how much of ITS journal to consider;
/// this one decides how much of the ANSWER reaches whatever is reading this
/// tool's stdout, and `--full` is this tool's own escape hatch, independent
/// of whatever the command itself asked the machine for.
pub const CAP_BYTES: usize = 65_536;

/// Cap `answer` to [`CAP_BYTES`], or return it unchanged when `full` was
/// asked for or the answer already fits.
///
/// **The notice names the real size and the flag that gets it all** — a cut
/// silent about its own existence is `AGENTS.md` §5's failure in miniature: a
/// truncated answer that does not say so reads as the whole answer.
///
/// Cut on the last UTF-8 character boundary at or before the cap, never mid
/// character: `call_tool`'s answer is the machine's own JSON, and a broken
/// multi-byte sequence at the tail would make even the KEPT portion fail to
/// parse.
#[must_use]
fn cap_answer(answer: &str, full: bool) -> String {
    if full || answer.len() <= CAP_BYTES {
        return answer.to_owned();
    }
    let mut boundary = CAP_BYTES;
    while !answer.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!(
        "{}\n\n--- CUT: {} of {} bytes shown. This tool's own cap, not the machine's \
         answer ({CAP_BYTES} bytes here; `--full` prints all of it). If the command itself \
         has a way to ask for less — `get log`'s limit, for one — that is usually the better \
         fix. ---\n",
        &answer[..boundary],
        boundary,
        answer.len(),
    )
}

/// **Whether stdout is a regular file**: `cargo rivet run … > answer.json`.
///
/// The cap protects whatever reads this tool's stdout as it arrives, a terminal
/// or an agent's captured pipe. A regular file is read by nobody while it is
/// written, and a cut there leaves JSON that does not parse, with the notice
/// after it where a parser has already failed. So a file gets the whole answer.
///
/// Asked of a duplicate of the descriptor, which is closed again; any error is
/// "not a regular file", so the cap stays on whenever this cannot tell.
#[cfg(unix)]
fn stdout_is_a_regular_file() -> bool {
    use std::os::fd::AsFd as _;
    std::io::stdout()
        .as_fd()
        .try_clone_to_owned()
        .map(std::fs::File::from)
        .and_then(|file| file.metadata())
        .is_ok_and(|meta| meta.is_file())
}

/// The same question, asked of Windows with `GetFileType`.
///
/// **Not `metadata().is_file()`, which answers yes for a pipe here**: on
/// Windows it means only "not a directory and not a link", so a pipe into
/// another program read as a file and was never cut. `GetFileType` is the call
/// that tells a file on a disk from a pipe (`FILE_TYPE_PIPE`) or a console
/// (`FILE_TYPE_CHAR`). It is declared here rather than taken from a crate:
/// one read-only function of `kernel32`, which every Windows process links.
#[cfg(windows)]
#[allow(unsafe_code)]
fn stdout_is_a_regular_file() -> bool {
    use std::os::windows::io::AsRawHandle as _;

    /// `winbase.h`'s answer for a file on a disk.
    const FILE_TYPE_DISK: u32 = 0x0001;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileType(file: *mut core::ffi::c_void) -> u32;
    }

    let handle = std::io::stdout().as_raw_handle();
    // SAFETY: `GetFileType` reads the type of the handle it is given and
    // writes nothing, and `handle` is this process's own standard output. A
    // closed or invalid handle is answered with `FILE_TYPE_UNKNOWN`, which is
    // "not a file" here, so the cap stays on.
    let kind = unsafe { GetFileType(handle) };
    kind == FILE_TYPE_DISK
}

/// No way to ask here, so the cap stays on.
#[cfg(not(any(unix, windows)))]
fn stdout_is_a_regular_file() -> bool {
    false
}

/// What `run` was asked to do: which machine, and which command.
#[derive(Debug)]
pub struct RunArgs {
    pub endpoint: Endpoint,
    /// The words after `--`, untouched.
    pub words: Vec<String>,
    /// Print the call and send nothing. See [`show`].
    pub show: bool,
    /// Do not cap the printed answer. See [`cap_answer`].
    pub full: bool,
}

/// **The call this would send, printed instead of sent.**
///
/// It is not a courtesy. The translation is the part of this path that has no
/// board in it, and the thing that closes it is a person or an agent being able
/// to see the exact `arguments` object and compare it to what a real
/// `rivet mcp` reads — which is a check that needs no hardware, because
/// `rivet mcp` is a stdio program and this workspace builds it.
///
/// It also needs no identity and no pin: nothing about a translation depends on
/// which machine it is for, so this works with `--host` alone or with none.
fn show(call: &rivet_remote::command::ToolCall) -> String {
    format!(
        "tool       {}\n\
         arguments  {}\n\
         words      rivet {} {}\n\
         \n\
         The machine binds `arguments` back to that word vector, in the order the\n\
         catalogue declares the inputs, and performs it. Nothing was sent.\n",
        call.tool,
        call.arguments,
        call.verb,
        call.words().join(" "),
    )
}

/// **Everything before `--` is this tool's; everything after it is the
/// machine's.**
///
/// The separator is required rather than inferred. A command in the machine's
/// vocabulary contains words that look like flags — `--expect`, `--append`,
/// `ADR-012` §7.3's `--accept` — and a parser that guessed where the flags
/// stopped would eat one of them exactly often enough to be trusted.
///
/// # Errors
///
/// No separator, a flag with no value, a flag this verb does not take, or
/// either half of `ADR-025` §2's trust without the other.
pub fn parse(args: &[String]) -> Result<RunArgs, String> {
    let at = args.iter().position(|a| a == "--").ok_or_else(|| {
        format!(
            "`--` is required, and the command goes after it.\n\
             \x20 why  a Rivet command contains words that look like this tool's flags — \
             `--expect`, `--append`, `--accept` — so where one ends and the other begins is \
             said rather than guessed.\n\n{}",
            usage()
        )
    })?;

    let mut endpoint = Endpoint::default();
    let mut show = false;
    let mut full = false;
    let mut index = 0;
    while index < at {
        if args[index] == "--show" {
            show = true;
            index += 1;
            continue;
        }
        if args[index] == "--full" {
            full = true;
            index += 1;
            continue;
        }
        match endpoint.read(&args[..at], index)? {
            Some(next) => index = next,
            None => {
                return Err(format!(
                    "{:?} is not an option `run` takes. Everything after `--` is the command; \
                     everything before it is --host, --identity, --pin, --show and --full.\n\n{}",
                    args[index],
                    usage()
                ));
            }
        }
    }
    endpoint.paired()?;

    let words = args[at + 1..].to_vec();
    if words.is_empty() {
        return Err(format!(
            "nothing follows `--`, so there is no command to run.\n\n{}",
            usage()
        ));
    }

    Ok(RunArgs {
        endpoint,
        words,
        show,
        full,
    })
}

/// `run`, with a real connection behind it.
///
/// # What it returns, and why a refusal is not dressed up
///
/// `Ok` carries the machine's answer and a newline, and nothing else — no
/// heading, no summary (`AGENTS.md` §5). `Err` carries **the machine's own
/// refusal text, unaltered**, which `main` prints on stderr behind
/// `cargo-rivet: `; that prefix is this tool naming itself and is the only thing
/// added to it anywhere. A refusal this end owns — an untranslatable command —
/// comes back the same way and is correctly attributed by that prefix, because
/// it is this tool's own sentence.
///
/// # The command is translated BEFORE a key is read or a socket is opened
///
/// Nothing about the translation depends on the machine, so a command this end
/// cannot send is refused without reading an identity, without a pin and
/// without a board — which is also what makes that refusal testable here.
///
/// # Errors
///
/// This end's refusal to translate, a missing transport, an identity or pin
/// that cannot be read, or the machine's own refusal.
pub fn execute(args: &[String]) -> Result<String, String> {
    // `PLAN-Feature-Set-2026-09-12` §7.1: a bare leading device name is
    // resolved against `rivet-fleet`'s own register into `--host`,
    // `--identity` and `--pin` BEFORE this tool's own parser ever runs, so
    // `parse` below is unchanged and every one of its existing tests still
    // exercises exactly the grammar it always has.
    let expanded = crate::device::expand_from_default(args)?;
    let parsed = parse(&expanded)?;
    let words: Vec<&str> = parsed.words.iter().map(String::as_str).collect();
    let call = rivet_remote::command::translate(&words)?;
    if parsed.show {
        return Ok(show(&call));
    }

    let transport = parsed.endpoint.open()?.ok_or_else(|| {
        format!(
            "--identity and --pin are required: `run` speaks ADR-035's control plane and has \
             no second transport to fall back to.\n\n{}",
            usage()
        )
    })?;
    transport
        .call_tool(&call.tool, &call.arguments)
        .map(|answer| format!("{}\n", cap_answer(&answer, parsed.full || stdout_is_a_regular_file())))
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn parse_of(list: &[&str]) -> Result<RunArgs, String> {
        let owned: Vec<String> = list.iter().map(|w| (*w).to_string()).collect();
        parse(&owned)
    }

    #[test]
    fn the_command_is_everything_after_the_separator_and_the_flags_everything_before() {
        let parsed = parse_of(&[
            "--host", "pi:7777", "--identity", "./id", "--pin", "ab", "--", "get", "storage",
        ])
        .expect("valid");
        assert_eq!(parsed.words, ["get", "storage"]);
        assert_eq!(parsed.endpoint.host.as_deref(), Some("pi:7777"));
    }

    /// **A flag word in the command is the machine's, not this tool's.** This
    /// is the whole reason `--` is mandatory.
    #[test]
    fn a_flag_word_after_the_separator_is_part_of_the_command() {
        let parsed = parse_of(&[
            "--host", "pi:7777", "--identity", "./id", "--pin", "ab", "--", "apply", "edit",
            "/p", "--expect", "a", "--replace", "b", "confirm",
        ])
        .expect("valid");
        assert_eq!(
            parsed.words,
            [
                "apply", "edit", "/p", "--expect", "a", "--replace", "b", "confirm"
            ]
        );
    }

    #[test]
    fn without_the_separator_it_refuses_and_says_why_rather_than_guessing() {
        let refused =
            parse_of(&["--host", "pi:7777", "get", "storage"]).expect_err("no separator");
        assert!(refused.contains("`--` is required"), "{refused}");
        assert!(refused.contains("--accept"), "{refused}");
    }

    #[test]
    fn a_separator_with_nothing_after_it_is_refused() {
        let refused = parse_of(&["--host", "pi:7777", "--"]).expect_err("no command");
        assert!(refused.contains("no command to run"), "{refused}");
    }

    #[test]
    fn an_option_this_verb_does_not_take_is_refused_rather_than_passed_through() {
        let refused =
            parse_of(&["--host", "pi:7777", "--rvt", "x.rvt", "--", "get", "storage"])
                .expect_err("not an option here");
        assert!(refused.contains("--rvt"), "{refused}");
    }

    /// `ADR-025` §2's pairing is checked here too, and by the same code
    /// `deploy` uses — `rivet_remote::endpoint`.
    #[test]
    fn half_the_trust_model_is_refused_before_anything_connects() {
        let refused = parse_of(&["--host", "pi:7777", "--identity", "./id", "--", "get", "storage"])
            .expect_err("no pin");
        assert!(refused.contains("--pin"), "{refused}");
    }

    /// **No transport is a refusal that names both flags**, rather than a
    /// silent fall back to some other channel.
    #[test]
    fn a_command_with_no_identity_refuses_instead_of_reaching_for_another_channel() {
        let owned: Vec<String> = ["--host", "pi:7777", "--", "get", "storage"]
            .iter()
            .map(|w| (*w).to_string())
            .collect();
        let refused = execute(&owned).expect_err("there is no transport");
        assert!(refused.contains("--identity and --pin are required"), "{refused}");
        assert!(refused.contains("no second transport"), "{refused}");
    }

    /// **A command this end cannot translate never reaches a connection.** It
    /// is refused with no identity, no pin and no board — which is why this
    /// test can run at all.
    #[test]
    fn an_untranslatable_command_is_refused_by_name_before_anything_is_opened() {
        let owned: Vec<String> = ["--host", "pi:7777", "--", "set", "hostname", "pi"]
            .iter()
            .map(|w| (*w).to_string())
            .collect();
        let refused = execute(&owned).expect_err("`set` has no operation");
        assert!(refused.contains("`set` is a Rivet verb"), "{refused}");
        assert!(refused.contains("rivet_set"), "{refused}");
        // And it is the TRANSLATION that refused, not the missing transport:
        // the order matters, because reading a key for a command that could
        // never be sent is work nobody asked for.
        assert!(!refused.contains("--identity and --pin are required"), "{refused}");
    }

    /// The usage names every flag it takes and the separator, so `cargo rivet
    /// run --help` is enough to use it.
    #[test]
    fn the_help_names_the_three_flags_the_separator_and_where_the_catalogue_is() {
        let usage = usage();
        for word in [
            "--host",
            "--identity",
            "--pin",
            "--show",
            "--",
            "rivet --version",
            "control plane",
        ] {
            assert!(usage.contains(word), "{word} missing from:\n{usage}");
        }
    }

    /// **Every port this help text shows is the control plane's own.**
    ///
    /// `PLAN.md` §92's fourth defect was three ports in three places — the
    /// README said `7423`, `rivet-mcpd --help` said `7777`, the board listened
    /// on `7777` — and an operator copying an example had no way to know which
    /// was real. Two of the three were wrong.
    ///
    /// **This scans rather than asserting one line.** Fixing the two examples
    /// and leaving the next one to be added wrong is `AGENTS.md` §7's failure:
    /// the number comes from `rivet_config::control_plane::DEFAULT_PORT`, and
    /// any OTHER port appearing anywhere in this usage fails here — including
    /// in an example nobody has written yet.
    #[test]
    fn every_port_in_the_help_is_the_control_planes_default() {
        let usage = usage();
        let default = rivet_config::control_plane::DEFAULT_PORT.to_string();
        let mut found = 0usize;
        // `host:port` in prose: a colon, then digits.
        for (at, _) in usage.match_indices(':') {
            let digits: String = usage[at + 1..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            if digits.is_empty() {
                continue;
            }
            found += 1;
            assert_eq!(
                digits, default,
                "this help shows port {digits}, and the control plane's is {default}:\n{usage}"
            );
        }
        // A scan that scanned nothing would pass in silence, which is the one
        // way this test could stop meaning anything.
        assert!(found >= 2, "no port was found in the usage at all:\n{usage}");
    }

    /// **`--show` needs nothing but the words.** No identity, no pin, no
    /// machine — which is what makes it usable to check a command before it
    /// touches anything, and what makes this test possible at all.
    #[test]
    fn show_prints_the_call_and_sends_nothing_with_no_identity_and_no_machine() {
        let owned: Vec<String> = ["--show", "--", "get", "storage"]
            .iter()
            .map(|w| (*w).to_string())
            .collect();
        let printed = execute(&owned).expect("a translation needs no machine");
        assert!(printed.contains("rivet_get"), "{printed}");
        assert!(printed.contains(r#"{"subject":"storage"}"#), "{printed}");
        assert!(printed.contains("rivet get storage"), "{printed}");
        assert!(printed.contains("Nothing was sent"), "{printed}");
    }

    /// And it still refuses what cannot be translated, rather than printing a
    /// call that does not exist.
    #[test]
    fn show_refuses_a_command_it_could_not_send_either() {
        let owned: Vec<String> = ["--show", "--", "set", "hostname", "pi"]
            .iter()
            .map(|w| (*w).to_string())
            .collect();
        assert!(execute(&owned).is_err_and(|e| e.contains("rivet_set")));
    }

    // ---- `--full`, and `parse` reading it ----------------------------------

    #[test]
    fn full_is_read_as_a_flag_before_the_separator() {
        let parsed = parse_of(&["--full", "--host", "pi:7777", "--", "get", "storage"])
            .expect("valid");
        assert!(parsed.full);
    }

    #[test]
    fn full_defaults_to_false() {
        let parsed = parse_of(&["--host", "pi:7777", "--", "get", "storage"]).expect("valid");
        assert!(!parsed.full);
    }

    // ---- `cap_answer` — `PLAN-Feature-Set-2026-09-12` §7.2's truncation ----

    /// **The property this cap exists for**, broken deliberately first
    /// (`AGENTS.md` §4): before the cut, this printed all 200 000 bytes and an
    /// agent reading it paid for every one of them.
    #[test]
    fn an_answer_over_the_cap_is_cut_and_says_the_real_size() {
        let huge = "a".repeat(CAP_BYTES * 3);
        let cut = cap_answer(&huge, false);
        assert!(cut.len() < huge.len(), "nothing was cut: {}", cut.len());
        assert!(cut.contains(&huge.len().to_string()), "{cut}");
        assert!(cut.contains("--full"), "{cut}");
    }

    #[test]
    fn an_answer_at_or_under_the_cap_is_not_touched() {
        let fits = "a".repeat(CAP_BYTES);
        assert_eq!(cap_answer(&fits, false), fits);
        let small = "get storage says this";
        assert_eq!(cap_answer(small, false), small);
    }

    /// `--full` is the escape hatch, and it bypasses this tool's own cap
    /// regardless of size.
    #[test]
    fn full_bypasses_the_cap_no_matter_how_large_the_answer() {
        let huge = "a".repeat(CAP_BYTES * 3);
        assert_eq!(cap_answer(&huge, true), huge);
    }

    /// **Cut on a real UTF-8 boundary, never mid character.** The answer is
    /// JSON that may carry non-ASCII text (a device name, a label), and a cut
    /// that split a multi-byte character would make even the KEPT portion fail
    /// to parse as UTF-8 at all.
    #[test]
    fn the_cut_lands_on_a_utf8_character_boundary() {
        // A three-byte character straddling the boundary: pad up to one byte
        // before the cap with ASCII, then place a multi-byte character across
        // the line.
        let mut huge = "a".repeat(CAP_BYTES - 1);
        huge.push('€'); // 3 bytes in UTF-8
        huge.push_str(&"b".repeat(1000));
        let cut = cap_answer(&huge, false);
        // Extract the kept prefix (everything before the notice).
        let kept = cut.split("\n\n--- CUT:").next().expect("a notice follows the kept text");
        assert!(kept.is_char_boundary(kept.len()), "{kept:?}");
        assert!(
            std::str::from_utf8(kept.as_bytes()).is_ok(),
            "the kept portion is not valid UTF-8"
        );
    }

    /// The usage text names `--full` and the cap it bypasses.
    #[test]
    fn the_help_names_full_and_the_cap() {
        let usage = usage();
        assert!(usage.contains("--full"), "{usage}");
        assert!(usage.contains(&CAP_BYTES.to_string()), "{usage}");
    }
}
