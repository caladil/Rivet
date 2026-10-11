//! How this process was started, and what that means.
//!
//! `rivet` is named in `/etc/shells` and is therefore exec'd as a login shell,
//! in shapes an ordinary CLI never sees. Measured on a Raspberry Pi 4 against
//! a purpose-built binary that printed its own `argv`:
//!
//! | What was asked for | What the login shell was exec'd as |
//! | :-- | :-- |
//! | a remote login, no pty | `argv[0] = "-ferrifake"`, `argc = 1` |
//! | a remote login with a pty | `argv[0] = "-ferrifake"`, `argc = 1` — identical |
//! | one remote command | `ferrifake -c "cmd"` |
//! | a legacy file copy | `ferrifake -c "scp -t /tmp/"` |
//! | a modern file copy | `ferrifake -c "/usr/lib/sftp-server"` |
//!
//! Note what the table shows that is easy to assume otherwise: **the leading `-`
//! appears only on the interactive rows.** A `-c` invocation carries an
//! undecorated `argv[0]`, so the dash is a signal about *interactivity* and not
//! about being a login shell, and [`classify`] treats it that way.
//!
//! ## The three behaviours, and the one that is not a behaviour
//!
//! 1. **Interactive** — refused. `ADR-005` §5.1 rules `nushell` the
//!    interactive login shell and rejects the Talos model of making `rivet`
//!    the shell with no general shell at all, *"because it leaves an operator
//!    able to do nothing the admin surface does not already expose"*. This
//!    binary must therefore **not** grow a prompt: doing so would foreclose the
//!    ruled design by satisfying its requirement badly.
//! 2. **`-c <string>`** — [`words`] and [`self_invocation`] below.
//! 3. **Anything else** — ordinary CLI arguments.
//!
//! ## Saying nothing on stdout is a correctness requirement, not a style
//!
//! The same measurement again, and this one cost a debugging session on hardware:
//!
//! > The test binary printed a single greeting line, and `scp` failed with
//! > `Received message too long 1717924466`. That integer is `0x66657272` — the
//! > ASCII bytes `ferr` … being read as an SFTP packet length.
//!
//! So every refusal in this crate is written to **stderr**, and stdout carries
//! only a value a command actually produced. `crate::run` returns the two
//! separately for exactly this reason, and
//! `a_refusal_writes_nothing_at_all_to_stdout` walks every refusal there is.
//!
//! ## Why there is no word splitting here
//!
//! `-c` is handed a string that a POSIX shell would tokenise: quotes, escapes,
//! variable expansion, globbing, operators, here-documents. `ADR-005` §5.4's
//! ruling is *"complete or absent — never partial"*, on the argument that a
//! compatibility environment which is incomplete **fails in ways that are
//! attributed to the caller rather than to the environment**.
//!
//! Approximating that tokeniser would be the same mistake one layer down. So
//! [`words`] does not approximate it: it **refuses** any string containing a
//! character a shell would treat specially, and splits only strings where
//! splitting on spaces and tabs is provably the same answer a shell would give.
//! There is no input on which this guesses. `AGENTS.md` §5 — refuse rather than
//! guess, because *"a wrong guess does not fail — it succeeds"*.
//!
//! ### One construct is reproduced rather than refused — 2026-08-25
//!
//! **`ADR-005` §5.4.1.3 rules single-quoted strings in.** Read the paragraph
//! above carefully and it is what *admits* them: the rule is not *refuse
//! everything a shell knows about*, it is *split only where splitting is
//! provably the answer a shell would give*. A single-quoted string in `sh` has
//! no escapes and no expansion — every byte between the quotes is literal, and
//! the only thing that ends it is the next `'`. There is nothing left to
//! approximate, and nothing to guess.
//!
//! The finding that forced it is `ADR-012` §8.3: **the maintainer's own worked
//! example could not be typed as a remote command**, because a value with a
//! space in it needs quoting and quoting was refused. Nor could most edits
//! worth making.
//!
//! Everything else in [`SHELL_METACHARACTERS`](rivet_core::shellwords::SHELL_METACHARACTERS) stays refused, and the reason
//! is unchanged for every one of them — reproducing `"`, `$`, `` ` `` or `\`
//! means reproducing a shell's *behaviour* rather than a construct that has
//! none.

use crate::refusal::{Code, Refusal};

/// Where the image installs this binary, and the value `/etc/shells` names.
///
/// **This is a second copy of a fact**, which `ADR-011` forbids: the first is
/// `CLI_PATH` in `cmd/rivet-image/src/system.rs`. It is duplicated for exactly
/// the reason `SUPERVISOR_PATH` and `INIT_PATH` already are — a binary crate has
/// no library target, so nothing can import from it — and it joins them in
/// a standing debt, tracked since 2026-08-23. `cmd/rivet-image`'s
/// `the_cli_path_matches_the_one_the_cli_answers_to` is the tripwire meanwhile.
pub const INSTALL_PATH: &str = "/system/bin/rivet";

/// The bare name, for a `-c` string that says `rivet …` with no path.
pub const SELF_NAME: &str = "rivet";

/// The two constants this module used to own.
///
/// **Re-exported from `rivet_core::shellwords` since 2026-08-26**, where the
/// splitting itself now lives. They are re-exported rather than deleted
/// because this module's own documentation and tripwires cite them by these
/// names, and because a reader of a login shell should be able to find *what
/// a shell would interpret* in the file that refuses it.
///
/// The reasoning for each absence — `=`, and the quote that left the list on
/// 2026-08-25 — travelled with them and is on the core constant.
pub use rivet_core::shellwords::QUOTE;
#[cfg(test)]
use rivet_core::shellwords::SHELL_METACHARACTERS;

/// What the process was asked to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invocation {
    /// Exec'd as a login shell with nothing to do: a human wants a prompt.
    InteractiveLogin,
    /// `-c <string>` — one remote command, or a file copy asking for its helper.
    Command(String),
    /// Ordinary invocation. These are the CLI's own arguments.
    Arguments(Vec<String>),
}

/// Decide which of the three this is.
///
/// Takes `argv[0]` and the rest separately rather than reading
/// `std::env::args`, which is what makes every branch reachable from a test on
/// any platform — `cmd/rivet-netd`'s `args` module takes the same shape for
/// the same reason.
pub fn classify(argv0: &str, args: &[String]) -> Result<Invocation, Refusal> {
    if args.first().map(String::as_str) == Some("-c") {
        return match args.len() {
            1 => Err(Refusal::new(
                Code::ARGUMENTS,
                "-c needs a command string. Whatever exec'd this as a login shell supplies \
                 one; if you typed this, you probably want `rivet --help`.",
            )),
            2 => Ok(Invocation::Command(args[1].clone())),
            // `sh -c 'cmd' name arg…` assigns the extra words to $0, $1 … There
            // are no positional parameters here to assign them to, and dropping
            // them would run a command that is not the one that was asked for.
            _ => Err(Refusal::about(
                Code::NotAShell,
                args[2..].join(" "),
                "-c takes exactly one command string. A shell would assign the words after \
                 it to $0, $1 … and rivet has no positional parameters to assign them to \
                 (ADR-005 §5.4).",
            )),
        };
    }

    // The login convention: `argv[0]` prefixed with `-`. It is consulted only
    // when there is nothing else to go on — a login shell handed arguments is
    // being asked to do something, and doing it is more useful than refusing on
    // the strength of a character in `argv[0]`.
    if is_login_argv0(argv0) && args.is_empty() {
        return Ok(Invocation::InteractiveLogin);
    }

    Ok(Invocation::Arguments(args.to_vec()))
}

/// The `-` convention, and nothing more is read into it.
pub fn is_login_argv0(argv0: &str) -> bool {
    argv0.starts_with('-')
}

/// Split a `-c` string, or refuse it. **Never guess** — see the module docs.
///
/// # Single-quoted strings — `ADR-005` §5.4.1.3, ruled 2026-08-25
///
/// **This is one more construct, not a relaxation of the rule.** The rule is
/// *split only where splitting is provably the answer a shell would give*, and
/// a single-quoted string in `sh` satisfies it exactly: there are **no escapes
/// and no expansion** inside one, every byte is literal, and the only thing
/// that ends it is the next `'`. There is no shell behaviour left to
/// approximate, which is precisely what is not true of `"`, `$` or `\`.
///
/// So inside a pair of quotes, *everything* is taken as it is — spaces, tabs,
/// metacharacters, and control characters including a newline. That last one
/// is deliberate and is what `ADR-012` §8.3 wanted a base64 spelling for: a
/// value with a trailing newline is now typeable.
///
/// **Quotes bind to a word, not to an argument.** `--expect='a b'` is one word,
/// the way a shell reads it, and `''` is a legitimately empty argument rather
/// than nothing at all — which is why this cannot simply filter out the empty
/// strings the way it used to.
///
/// **An unterminated quote is refused.** A shell would go on reading; this
/// process has one string and nowhere to read from, and guessing where the
/// operator meant it to close is the one thing this module never does.
pub fn words(command: &str) -> Result<Vec<String>, Refusal> {
    // **The splitting itself is `rivet_core::shellwords`'s**, since 2026-08-26.
    // It moved there when `cmd/rivet-gitd` became a second program that stands
    // between a network and an exec with one string to read; two parsers that
    // disagreed about where a quote ends is the shape a confused-deputy bug
    // takes. What stays here is the *vocabulary* — this binary's exit codes and
    // the sentences an operator of a login shell needs.
    let words = rivet_core::shellwords::split(command).map_err(|e| match e {
        rivet_core::shellwords::SplitError::Metacharacter { found } => Refusal::about(
            Code::NotAShell,
            command,
            format!(
                "the command contains {}, which a shell would interpret. rivet is \
                 not a shell and will not guess what it means (ADR-005 §5.4: \
                 complete or absent, never partial). A value with spaces in it goes \
                 in SINGLE quotes, which are exactly reproducible and are accepted \
                 (ADR-005 §5.4.1.3).",
                rivet_core::shellwords::describe(found)
            ),
        ),
        rivet_core::shellwords::SplitError::Unterminated => Refusal::about(
            Code::NotAShell,
            command,
            format!(
                "the command has an opening {QUOTE} and no closing one. A shell would keep \
                 reading until it found the other half; this process was handed one string \
                 and has nowhere to read from, and guessing where the quote was meant to \
                 close is the one thing it will not do (ADR-005 §5.4.1.3)."
            ),
        ),
    })?;

    if words.is_empty() {
        return Err(Refusal::new(
            Code::ARGUMENTS,
            "-c was given an empty command string. A shell would exit 0 having done \
             nothing; rivet refuses rather than reporting success for work it did not do \
             (AGENTS.md §3).",
        ));
    }
    Ok(words)
}

/// Name a character in a way that survives being printed to a console.
fn describe(c: char) -> String {
    match c {
        '\n' => "a newline".to_string(),
        '\r' => "a carriage return".to_string(),
        // No arm for `\t`: `words` does not refuse it, so an arm here would be
        // a branch nothing can reach — see `SHELL_METACHARACTERS`.
        c if c.is_control() => format!("the control character U+{:04X}", c as u32),
        c => format!("{c:?}"),
    }
}

/// Decide whether a `-c` command line is asking for **this** program, and hand
/// back its arguments if so.
///
/// The only program `rivet` can run is itself, and it does not run it — it
/// *is* it, so a `-c rivet …` is answered directly with no `exec` anywhere.
/// Anything else is refused by name, and that refusal is the one an operator
/// meets first: `scp` and `sftp` both resolve to
/// `-c "/usr/lib/sftp-server"`, so file transfer to a
/// Rivet host fails here, deliberately and with a message that names the
/// binary and the ruling that will supply it (`PLAN.md` §4.26).
///
/// **A path is matched exactly, never by basename.** `/tmp/rivet` is somebody
/// else's binary and answering as though it were this one would be a lie about
/// which program ran.
pub fn self_invocation(words: &[String]) -> Result<Vec<String>, Refusal> {
    let Some((program, rest)) = words.split_first() else {
        return Err(Refusal::new(
            Code::ARGUMENTS,
            "-c was given an empty command string",
        ));
    };
    if program == SELF_NAME || program == INSTALL_PATH {
        return Ok(rest.to_vec());
    }
    Err(Refusal::about(
        Code::NotAShell,
        program.clone(),
        format!(
            "rivet cannot execute programs. It is the login shell and \
             the only command it answers is `{SELF_NAME}` (or `{INSTALL_PATH}`). Note that \
             scp and sftp both ask for /usr/lib/sftp-server, which this image does not \
             ship — PLAN.md §4.26 rules how a file will reach a Rivet host."
        ),
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_string()).collect()
    }

    // -- the observed login-shell invocations ---------------------------------
    //
    // Every case below is a row of a table captured from a
    // Raspberry Pi 4 — provenance stated per AGENTS.md §4, because a
    // hand-composed fixture would prove only that this code agrees with its own
    // author.

    #[test]
    fn a_remote_login_with_no_command_is_an_interactive_login() {
        assert_eq!(
            classify("-rivet", &[]),
            Ok(Invocation::InteractiveLogin),
            "argv[0] = \"-rivet\", argc = 1"
        );
    }

    /// Asking for a pty produces the identical `argv`, which is the point: a
    /// pty is not distinguishable here and must not be guessed at.
    #[test]
    fn a_pty_login_is_indistinguishable_from_one_without_and_is_not_guessed_at() {
        assert_eq!(classify("-rivet", &[]), classify("-rivet", &[]));
    }

    #[test]
    fn a_remote_command_arrives_as_dash_c_with_an_undecorated_argv0() {
        assert_eq!(
            classify("rivet", &argv(&["-c", "rivet get x"])),
            Ok(Invocation::Command("rivet get x".to_string()))
        );
    }

    #[test]
    fn legacy_scp_asks_for_scp_minus_t() {
        let Ok(Invocation::Command(c)) = classify("rivet", &argv(&["-c", "scp -t /tmp/"])) else {
            panic!("scp -O did not classify as a command");
        };
        let refusal = words(&c).and_then(|w| self_invocation(&w));
        assert!(matches!(
            refusal,
            Err(Refusal {
                code: Code::NotAShell,
                ..
            })
        ));
    }

    /// The row that costs the most to misread: modern `scp` and `sftp` both ask
    /// for a binary this image does not contain, and the refusal must name it.
    #[test]
    fn sftp_asks_for_a_server_binary_and_is_refused_by_name() {
        let Ok(w) = words("/usr/lib/sftp-server") else {
            panic!("the sftp-server path was refused as unsplittable");
        };
        let Err(r) = self_invocation(&w) else {
            panic!("rivet claimed it could run sftp-server");
        };
        assert_eq!(r.code, Code::NotAShell);
        assert_eq!(r.subject.as_deref(), Some("/usr/lib/sftp-server"));
        assert!(r.message.contains("sftp-server"), "{}", r.message);
    }

    // -- the dash on argv[0] ---------------------------------------------------

    #[test]
    fn the_dash_is_the_login_convention_and_its_absence_is_an_ordinary_run() {
        assert!(is_login_argv0("-rivet"));
        assert!(is_login_argv0("-/system/bin/rivet"));
        assert!(!is_login_argv0("rivet"));
        assert!(!is_login_argv0("/system/bin/rivet"));
    }

    /// A login shell handed arguments is being asked to do something. Refusing
    /// on the strength of `argv[0]` would make `rivet` unusable from anything
    /// that sets the dash and then passes a command.
    #[test]
    fn a_login_argv0_with_arguments_is_still_an_ordinary_request() {
        assert_eq!(
            classify("-rivet", &argv(&["get", "x"])),
            Ok(Invocation::Arguments(argv(&["get", "x"])))
        );
    }

    #[test]
    fn an_ordinary_invocation_with_no_arguments_is_not_a_login() {
        assert_eq!(classify("rivet", &[]), Ok(Invocation::Arguments(vec![])));
    }

    // -- -c, and the refusal to guess -----------------------------------------

    #[test]
    fn dash_c_with_no_string_is_an_argument_error_rather_than_an_empty_command() {
        let Err(r) = classify("rivet", &argv(&["-c"])) else {
            panic!("-c with no string was accepted");
        };
        assert_eq!(r.code, Code::ARGUMENTS);
    }

    #[test]
    fn dash_c_with_extra_words_refuses_rather_than_dropping_them() {
        let Err(r) = classify("rivet", &argv(&["-c", "rivet get x", "name", "arg"])) else {
            panic!("the extra words after -c were silently dropped");
        };
        assert_eq!(r.code, Code::NotAShell);
        assert_eq!(r.subject.as_deref(), Some("name arg"));
    }

    /// **The heart of this module.** Each of these is a string a POSIX shell
    /// would tokenise differently from "split on spaces", so each must be
    /// refused rather than split — `ADR-005` §5.4, complete or absent.
    #[test]
    fn every_shell_construct_is_refused_rather_than_approximated() {
        for command in [
            r#"rivet get "two words""#,
            // ~~`"rivet get 'quoted'"`~~ — moved to
            // `a_single_quoted_string_is_one_word_with_its_spaces_intact`,
            // which asserts the opposite. `ADR-005` §5.4.1.3, 2026-08-25.
            "rivet get $HOME",
            "rivet get `hostname`",
            "rivet get a\\ b",
            "rivet get x | rivet get y",
            "rivet get x && rivet get y",
            "rivet get x; rivet get y",
            "rivet get x > /tmp/out",
            "rivet get x < /tmp/in",
            "(rivet get x)",
            "{ rivet get x; }",
            "rivet get *",
            "rivet get a?b",
            "rivet get a[bc]",
            "rivet get ~/x",
            "rivet get x # comment",
            "rivet get !!",
            "rivet get x\nrivet get y",
        ] {
            let got = words(command);
            assert!(
                matches!(
                    got,
                    Err(Refusal {
                        code: Code::NotAShell,
                        ..
                    })
                ),
                "{command:?} was tokenised instead of refused: {got:?}"
            );
        }
    }

    /// The refusal has to say *which* character, because the operator's next
    /// move is to retype the command without it.
    #[test]
    fn the_refusal_names_the_character_that_caused_it() {
        let Err(r) = words("rivet get $HOME") else {
            panic!("a dollar sign was tokenised");
        };
        assert!(r.message.contains('$'), "{}", r.message);
        assert_eq!(r.subject.as_deref(), Some("rivet get $HOME"));

        let Err(r) = words("rivet get x\ny") else {
            panic!("a newline was tokenised");
        };
        assert!(r.message.contains("newline"), "{}", r.message);
    }

    /// The strings that *are* splittable, and the answer must be the same one a
    /// shell would give — which is the only reason splitting them is honest.
    #[test]
    fn a_string_a_shell_would_split_the_same_way_is_split() {
        assert_eq!(words("rivet"), Ok(owned(&["rivet"])));
        assert_eq!(words("rivet get x"), Ok(owned(&["rivet", "get", "x"])));
        assert_eq!(
            words("  rivet \t get   x  "),
            Ok(owned(&["rivet", "get", "x"]))
        );
        assert_eq!(
            words("rivet --output=json get x"),
            Ok(owned(&["rivet", "--output=json", "get", "x"]))
        );
    }

    /// Every word a caller can produce, as the owned strings [`words`] hands
    /// back — it cannot borrow from its input any more, because a quoted word's
    /// content is not a contiguous slice of it.
    fn owned(w: &[&str]) -> Vec<String> {
        w.iter().map(|s| (*s).to_string()).collect()
    }

    // -- single-quoted strings — `ADR-005` §5.4.1.3 ---------------------------

    /// **The construct the whole ruling is about**, in the shape `ADR-012` §8.3
    /// could not type.
    #[test]
    fn a_single_quoted_string_is_one_word_with_its_spaces_intact() {
        assert_eq!(
            words("rivet apply edit f --expect 'show_banner = true'"),
            Ok(owned(&[
                "rivet",
                "apply",
                "edit",
                "f",
                "--expect",
                "show_banner = true",
            ]))
        );
    }

    /// A shell binds quotes to a **word**, not to an argument, so `--x='a b'`
    /// is one word and `a'b'c` is `abc`. Reproducing that is the whole claim.
    #[test]
    fn quotes_bind_to_a_word_the_way_a_shell_binds_them() {
        assert_eq!(
            words("rivet --expect='a b'"),
            Ok(owned(&["rivet", "--expect=a b"]))
        );
        assert_eq!(words("rivet a'b'c"), Ok(owned(&["rivet", "abc"])));
        assert_eq!(
            words("rivet 'a''b'"),
            Ok(owned(&["rivet", "ab"])),
            "two adjacent quoted runs are one word, as in sh"
        );
    }

    /// **Inside the quotes, everything is literal** — that is what a
    /// single-quoted string *is*, and it is why splitting one needs no
    /// judgement. A metacharacter here is data, not syntax.
    #[test]
    fn every_metacharacter_is_literal_inside_quotes() {
        assert_eq!(
            words("rivet set x '$HOME | rm -rf * ; #!~`\\'"),
            Ok(owned(&["rivet", "set", "x", "$HOME | rm -rf * ; #!~`\\"]))
        );
    }

    /// A newline inside quotes is legal in `sh` and is legal here, which is
    /// what `ADR-012` §8.3 wanted a base64 spelling for: a value with a
    /// trailing newline is typeable.
    #[test]
    fn a_newline_is_literal_inside_quotes_and_still_refused_outside_them() {
        assert_eq!(
            words("rivet set x 'line\n'"),
            Ok(owned(&["rivet", "set", "x", "line\n"]))
        );
        assert!(
            words("rivet set\nx").is_err(),
            "outside quotes a newline still separates two commands"
        );
    }

    /// `''` is an empty argument, and it is not the same as no argument. It is
    /// why the tokeniser cannot simply drop empty words the way it used to.
    #[test]
    fn an_explicitly_empty_argument_survives() {
        assert_eq!(
            words("rivet set x ''"),
            Ok(owned(&["rivet", "set", "x", ""]))
        );
    }

    /// A shell would keep reading for the other half. This process has one
    /// string and nowhere to read from, and guessing where the quote closes is
    /// the one thing this module never does.
    #[test]
    fn an_unterminated_quote_is_refused_rather_than_closed_for_the_operator() {
        let Err(r) = words("rivet set x 'unfinished") else {
            panic!("an unterminated quote was accepted");
        };
        assert_eq!(r.code, Code::NotAShell);
        assert!(r.message.contains("closing"), "{}", r.message);
    }

    /// **The other half of the ruling**, and the one that stops this becoming a
    /// shell one construct at a time. Each of these needs a shell's *behaviour*
    /// reproduced, not just a construct with none.
    #[test]
    fn every_other_shell_construct_is_still_refused() {
        for command in [
            "rivet set x \"a b\"",
            "rivet set x $HOME",
            "rivet set x `id`",
            "rivet set x a\\ b",
            "rivet get log | head",
            "rivet get log > f",
            "rivet get log; id",
            "rivet set x ~/f",
            "rivet set x *.toml",
        ] {
            assert!(
                matches!(
                    words(command),
                    Err(Refusal {
                        code: Code::NotAShell,
                        ..
                    })
                ),
                "{command:?} was tokenised instead of refused"
            );
        }
    }

    /// The tripwire named in [`QUOTE`]'s documentation: re-adding `'` to the
    /// metacharacter list would silently revert `ADR-005` §5.4.1.3, and this is
    /// what makes that fail instead.
    #[test]
    fn the_quote_is_not_in_the_refused_set() {
        assert!(
            !SHELL_METACHARACTERS.contains(QUOTE),
            "`'` is reproduced by `words`, not refused by it — ADR-005 §5.4.1.3"
        );
    }

    /// Tab is `IFS` whitespace and splits; every other control character is
    /// refused. The pair is one decision and is tested as one, because getting
    /// it half right is what the first version did — it refused the tab.
    #[test]
    fn tab_splits_and_every_other_control_character_refuses() {
        assert_eq!(words("rivet\tget\tx"), Ok(owned(&["rivet", "get", "x"])));
        for c in (0u32..0x20).chain(std::iter::once(0x7f)) {
            if c == 0x09 {
                continue;
            }
            let raw = char::from_u32(c).unwrap_or('?');
            let command = format!("rivet{raw}get");
            assert!(
                matches!(
                    words(&command),
                    Err(Refusal {
                        code: Code::NotAShell,
                        ..
                    })
                ),
                "U+{c:04X} was tokenised instead of refused"
            );
        }
    }

    /// `=` is not a shell metacharacter in an argument position, and refusing it
    /// would break `--output=json` for no reason.
    #[test]
    fn an_equals_sign_is_not_treated_as_special() {
        assert!(!SHELL_METACHARACTERS.contains('='));
    }

    #[test]
    fn an_empty_command_string_refuses_rather_than_succeeding_silently() {
        let Err(r) = words("   ") else {
            panic!("a blank command string was accepted");
        };
        assert_eq!(r.code, Code::ARGUMENTS);
    }

    // -- what -c may dispatch to ----------------------------------------------

    #[test]
    fn the_bare_name_and_the_install_path_are_the_two_accepted_programs() {
        assert_eq!(
            self_invocation(&owned(&["rivet", "get", "x"])),
            Ok(owned(&["get", "x"]))
        );
        assert_eq!(
            self_invocation(&owned(&["/system/bin/rivet", "--version"])),
            Ok(owned(&["--version"]))
        );
        assert_eq!(self_invocation(&owned(&["rivet"])), Ok(Vec::new()));
    }

    /// **A basename match would be a lie.** `/tmp/rivet` is a different file;
    /// answering as though it were this one would report that a program ran
    /// which never did.
    #[test]
    fn a_different_binary_that_happens_to_be_called_rivet_is_refused() {
        for program in [
            "/tmp/rivet",
            "./rivet",
            "/usr/bin/rivet",
            "rivet-netd",
        ] {
            let got = self_invocation(&owned(&[program]));
            assert!(
                matches!(
                    got,
                    Err(Refusal {
                        code: Code::NotAShell,
                        ..
                    })
                ),
                "{program} was answered as though it were this binary: {got:?}"
            );
        }
    }

    /// The two constants must agree with what the image installs. The other
    /// half of this pair lives in `cmd/rivet-image`, which is the crate that
    /// can see the tree.
    #[test]
    fn the_install_path_ends_in_the_name_that_is_answered_bare() {
        assert_eq!(INSTALL_PATH, "/system/bin/rivet");
        assert!(INSTALL_PATH.ends_with(&format!("/{SELF_NAME}")));
    }
}
