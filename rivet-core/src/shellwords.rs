//! **How a one-string command splits into words** — and the one place that is
//! decided.
//!
//! `ADR-005` §5.4: *complete or absent, never partial.* The rule is **split
//! only where splitting is provably the answer a shell would give**, and a
//! single-quoted string in `sh` satisfies it exactly: there are no escapes and
//! no expansion inside one, every byte is literal, and the only thing that ends
//! it is the next `'`. There is no shell behaviour left to approximate — which
//! is precisely what is not true of `"`, `$` or `\`.
//!
//! # Why this lives in the core
//!
//! It was `cmd/rivet`'s `invocation` module until 2026-08-26, and it moved on
//! the day the second consumer appeared: a second program that receives its
//! work as one whole command line in a single string, exactly as `cmd/rivet` receives
//! its own in `-c`.
//!
//! **Two implementations of this would be the worst possible `ADR-011`
//! violation**, because both are security boundaries. Two parsers that disagree
//! about where a quote ends is not a cosmetic inconsistency; it is the shape a
//! confused-deputy bug takes. So there is one, and both callers dress its
//! refusal in their own vocabulary.
//!
//! # What it refuses, and never guesses
//!
//! * **A shell metacharacter or a control character outside quotes.** A shell
//!   would interpret it; this does not, and will not guess what it meant.
//! * **An unterminated quote.** A shell would keep reading. A process handed
//!   one string has nowhere to read from, and guessing where the operator meant
//!   it to close is the one thing neither caller ever does.
//!
//! Inside a pair of quotes *everything* is taken as it is — spaces, tabs,
//! metacharacters, and control characters including a newline. That last one is
//! deliberate and is what `ADR-012` §8.3 wanted a base64 spelling for.
//!
//! **Quotes bind to a word, not to an argument.** `--expect='a b'` is one word,
//! the way a shell reads it, and `''` is a legitimately empty argument rather
//! than nothing at all.

/// The quote this understands, and the only one.
pub const QUOTE: char = '\'';

/// Everything a shell would act on, refused outside quotes.
///
/// **Moved verbatim from `cmd/rivet`'s `invocation` module**, character for
/// character, and that is the point of the move rather than an incidental of
/// it: a second list would be a second answer to *what a shell would
/// interpret*, in two programs that both stand between a network and an exec.
///
/// Two absences are decisions and are carried across with it:
///
/// * **`=` is deliberately absent.** It is not special to a shell in an
///   argument position, and `--output=json` is a form the CLI accepts.
/// * **The quote left this list on 2026-08-25** (`ADR-005` §5.4.1.3). It is
///   still a character that means something to a shell; it is now one whose
///   meaning this module *reproduces* rather than refuses, which is the
///   distinction the whole rule turns on. [`QUOTE`] holds it, and
///   `the_quote_is_not_also_a_refused_metacharacter` is what stops somebody
///   re-adding it and silently reverting that ruling.
///
/// A **tab** is not here: `sh` treats it as `IFS` whitespace exactly as it
/// treats a space, so splitting on it *is* the answer a shell would give. A
/// **newline** is not the same case and is refused by the control-character
/// rule - in a command string it separates two commands, and running the first
/// while dropping the second is the worst available outcome.
pub const SHELL_METACHARACTERS: &str = "\"`\\$|&;<>(){}*?[]~#!";

/// Why a command string could not be split.
///
/// **Two variants and no more**, because there are exactly two ways this can
/// fail and each needs a different sentence from its caller. A caller turns
/// these into its own refusal type; the core does not know about exit codes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitError {
    /// A character a shell would interpret, outside quotes.
    Metacharacter { found: char },
    /// An opening quote with no closing one.
    Unterminated,
}

/// Name a character the way a refusal should: an error names
/// what was wrong, not merely that something was.
#[must_use]
pub fn describe(c: char) -> String {
    match c {
        '\n' => "a newline".to_owned(),
        '\r' => "a carriage return".to_owned(),
        '\t' => "a tab".to_owned(),
        c if c.is_control() => format!("a control character (U+{:04X})", c as u32),
        c => format!("`{c}`"),
    }
}

/// Split a command string, or refuse it.
///
/// An **empty** result is possible and is not an error here: a caller that
/// requires at least one word says so in its own words, because "an empty
/// command" means something different to a login shell than it does to a
/// restricted one.
///
/// # Errors
///
/// [`SplitError`] — see the module header for what is refused and why nothing
/// is guessed.
pub fn split(command: &str) -> Result<Vec<String>, SplitError> {
    let mut words: Vec<String> = Vec::new();
    let mut word: Option<String> = None;
    let mut quoted = false;

    for c in command.chars() {
        if quoted {
            if c == QUOTE {
                quoted = false;
            } else {
                // Every byte, literally. The whole of what a single-quoted
                // string means in `sh`.
                word.get_or_insert_with(String::new).push(c);
            }
            continue;
        }
        match c {
            QUOTE => {
                quoted = true;
                // A word exists from the opening quote onwards, so that `''` is
                // an empty argument rather than no argument.
                word.get_or_insert_with(String::new);
            }
            ' ' | '\t' => {
                if let Some(done) = word.take() {
                    words.push(done);
                }
            }
            c if SHELL_METACHARACTERS.contains(c) || c.is_control() => {
                return Err(SplitError::Metacharacter { found: c });
            }
            c => word.get_or_insert_with(String::new).push(c),
        }
    }
    if quoted {
        return Err(SplitError::Unterminated);
    }
    if let Some(done) = word.take() {
        words.push(done);
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn split_ok(command: &str) -> Vec<String> {
        split(command).unwrap_or_else(|e| panic!("{command:?} was refused: {e:?}"))
    }

    #[test]
    fn an_ordinary_command_splits_on_whitespace() {
        assert_eq!(split_ok("rivet get log"), ["rivet", "get", "log"]);
        assert_eq!(split_ok("  rivet   get  "), ["rivet", "get"]);
        assert_eq!(split_ok(""), Vec::<String>::new());
    }

    /// **The construct that makes this reusable at all.** git sends
    /// `git-receive-pack '/path with spaces'`, and a path with a space in it is
    /// one word.
    #[test]
    fn a_single_quoted_string_is_one_word_with_its_spaces_intact() {
        assert_eq!(
            split_ok("git-receive-pack '/state/git/my repo.git'"),
            ["git-receive-pack", "/state/git/my repo.git"]
        );
    }

    /// Every byte is literal inside quotes — which is the property that makes
    /// splitting *provable* rather than approximate.
    #[test]
    fn every_metacharacter_is_literal_inside_quotes() {
        assert_eq!(split_ok("a '|&;<>()$`\\\"*?[]#~=%!{}'"), ["a", "|&;<>()$`\\\"*?[]#~=%!{}"]);
        assert_eq!(split_ok("a '\n'"), ["a", "\n"]);
    }

    /// **The two must never overlap**, or `ADR-005` §5.4.1.3 is silently
    /// reverted: a quote that is also a refused metacharacter is a quote that
    /// can never be typed.
    #[test]
    fn the_quote_is_not_also_a_refused_metacharacter() {
        assert!(!SHELL_METACHARACTERS.contains(QUOTE));
    }

    /// `=` is not special to a shell in an argument position, and the CLI
    /// accepts `--output=json`.
    #[test]
    fn an_equals_sign_is_not_a_metacharacter() {
        assert!(!SHELL_METACHARACTERS.contains('='));
        assert_eq!(split_ok("--output=json"), ["--output=json"]);
    }

    #[test]
    fn quotes_bind_to_a_word_the_way_a_shell_binds_them() {
        assert_eq!(split_ok("--expect='a b'"), ["--expect=a b"].map(String::from));
        assert_eq!(split_ok("a '' b"), ["a", "", "b"]);
    }

    /// A shell would interpret it; this refuses and names it.
    #[test]
    fn a_metacharacter_outside_quotes_is_refused_by_name() {
        for (command, found) in [
            ("a | b", '|'),
            ("a; b", ';'),
            ("a $HOME", '$'),
            ("a `b`", '`'),
            ("a > b", '>'),
        ] {
            assert_eq!(
                split(command),
                Err(SplitError::Metacharacter { found }),
                "{command:?}"
            );
        }
    }

    /// **The one thing neither caller ever does.** A shell would keep reading;
    /// a process handed one string has nowhere to read from.
    #[test]
    fn an_unterminated_quote_is_refused_rather_than_closed() {
        assert_eq!(split("a 'b"), Err(SplitError::Unterminated));
        assert_eq!(split("'"), Err(SplitError::Unterminated));
    }

    /// A tab separates, and is literal inside quotes — the asymmetry a caller
    /// would otherwise have to rediscover.
    #[test]
    fn a_tab_separates_outside_quotes_and_is_literal_inside_them() {
        assert_eq!(split_ok("a\tb"), ["a", "b"]);
        assert_eq!(split_ok("'a\tb'"), ["a\tb"]);
    }

    #[test]
    fn a_refusal_names_the_character_rather_than_only_reporting_one() {
        assert_eq!(describe('\n'), "a newline");
        assert_eq!(describe('|'), "`|`");
        assert!(describe('\u{7}').contains("U+0007"));
    }
}
