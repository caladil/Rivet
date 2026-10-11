//! **The one-time passkey enrollment code** — `ADR-063` §4.
//!
//! The first passkey on a machine is enrolled with a code issued over the
//! control plane (`rivet apply passkey-code confirm`), because the control
//! plane is already authenticated by pinned keys and a browser that holds no
//! passkey yet has nothing else to prove it is the operator's. Two programs
//! touch the code and neither may disagree with the other about it:
//!
//! * `rivet`, which mints it and writes it into `rivet-proxyd`'s data
//!   directory, owned by the proxy's uid, mode `0600`;
//! * `rivet-proxyd`, which reads it when a browser offers one, honours it for
//!   [`ENROLLMENT_LIFETIME_SECONDS`] from the file's modification time, and
//!   deletes it when a passkey is enrolled with it or when it has been guessed
//!   at [`ENROLLMENT_ATTEMPTS`] times.
//!
//! So the file's name, the lifetime, the alphabet and the normalisation live
//! here, once (`ADR-011`).
//!
//! **The file holds the code itself, not a hash of it.** It sits where the
//! proxy's TLS key sits, with the same owner and mode, and it is worth less than
//! that key: it expires in minutes and enrolls one passkey.

/// The file, inside `rivet-proxyd`'s data directory.
///
/// Spelled as machines already store it: renaming it would leave a code
/// issued by one build unreadable by the next.
pub const ENROLLMENT_FILE: &str = "passkey-enrolment";

/// **Where a person signs in** — `ADR-046` §4. The same path on every machine,
/// so `https://<machine>/sign-in` is one address a person learns to trust.
/// `rivet-proxyd` serves it, and no application may be mounted there
/// (`crate::package::application::RESERVED_PROXY_PATHS`).
pub const SIGN_IN_PAGE: &str = "/sign-in";

/// Where a passkey is enrolled, under [`SIGN_IN_PAGE`]; `rivet apply
/// passkey-code` sends the operator here with the code it issues.
pub const ENROLL_PAGE: &str = "/sign-in/enroll";

/// The service whose data directory holds it.
pub const PROXY_SERVICE: &str = "rivet-proxyd";

/// How long a code is honoured, from the file's modification time.
///
/// Ten minutes: long enough to walk from the terminal that issued it to a
/// browser on another device, short enough that one left lying in a
/// transcript is dead before anybody reads it.
pub const ENROLLMENT_LIFETIME_SECONDS: u64 = 10 * 60;

/// How many wrong codes the proxy takes before it deletes the file.
pub const ENROLLMENT_ATTEMPTS: u32 = 5;

/// The code's alphabet: Crockford's base 32, which has no `I`, `L`, `O` or
/// `U`, so a code read aloud or copied by hand has no look-alike pairs.
pub const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Characters in a code: fifty bits, against at most
/// [`ENROLLMENT_ATTEMPTS`] guesses in [`ENROLLMENT_LIFETIME_SECONDS`].
pub const CODE_CHARACTERS: usize = 10;

/// Random bytes a code is minted from — one per character, each reduced to
/// five bits, so every character is uniform over [`ALPHABET`].
pub const CODE_RANDOM_BYTES: usize = CODE_CHARACTERS;

/// **A code from random bytes**, in its canonical stored form: ten characters
/// of [`ALPHABET`], no separator. `None` when `random` is not
/// [`CODE_RANDOM_BYTES`] long.
#[must_use]
pub fn code_from(random: &[u8]) -> Option<String> {
    if random.len() != CODE_RANDOM_BYTES {
        return None;
    }
    Some(
        random
            .iter()
            .map(|byte| char::from(ALPHABET[usize::from(byte & 0x1f)]))
            .collect(),
    )
}

/// A canonical code as a person reads it: `ABCDE-FGHJK`.
#[must_use]
pub fn display(code: &str) -> String {
    if code.len() == CODE_CHARACTERS && code.is_ascii() {
        format!("{}-{}", &code[..5], &code[5..])
    } else {
        code.to_string()
    }
}

/// **What a person typed, in canonical form**, or `None` when it cannot be a
/// code. Case, spaces and hyphens are forgiven; Crockford's look-alikes are
/// folded (`I` and `L` to `1`, `O` to `0`). Anything else is refused rather
/// than guessed at.
#[must_use]
pub fn normalise(typed: &str) -> Option<String> {
    let mut out = String::with_capacity(CODE_CHARACTERS);
    for c in typed.chars() {
        if c == '-' || c.is_whitespace() {
            continue;
        }
        let c = match c.to_ascii_uppercase() {
            'I' | 'L' => '1',
            'O' => '0',
            other => other,
        };
        if !c.is_ascii() || !ALPHABET.contains(&(c as u8)) {
            return None;
        }
        out.push(c);
    }
    (out.len() == CODE_CHARACTERS).then_some(out)
}

/// **Whether a typed code matches the stored one**, comparing every byte
/// whatever the first difference, so the time an answer takes says nothing
/// about how much of a guess was right.
#[must_use]
pub fn matches(stored: &str, typed: &str) -> bool {
    let (Some(stored), Some(typed)) = (normalise(stored), normalise(typed)) else {
        return false;
    };
    stored
        .bytes()
        .zip(typed.bytes())
        .fold(0u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_code_is_ten_characters_of_the_alphabet() {
        let code = code_from(&[0, 1, 31, 32, 255, 7, 8, 9, 10, 200]).expect("ten bytes");
        assert_eq!(code.len(), CODE_CHARACTERS);
        assert!(code.bytes().all(|b| ALPHABET.contains(&b)));
        assert_eq!(&code[..5], "01Z0Z");
        assert_eq!(code_from(&[0; 9]), None);
        assert_eq!(code_from(&[0; 11]), None);
    }

    #[test]
    fn every_byte_value_lands_uniformly_on_the_alphabet() {
        let mut seen = [0u32; 32];
        for byte in 0..=255u8 {
            let code = code_from(&[byte; 10]).expect("ten");
            let at = ALPHABET.iter().position(|&a| a == code.as_bytes()[0]).expect("in");
            seen[at] += 1;
        }
        assert!(seen.iter().all(|&n| n == 8), "{seen:?}");
    }

    #[test]
    fn what_a_person_types_is_forgiven_only_where_it_cannot_mislead() {
        assert_eq!(normalise("abcde-fghjk").as_deref(), Some("ABCDEFGHJK"));
        assert_eq!(normalise(" ABCDE FGHJK ").as_deref(), Some("ABCDEFGHJK"));
        assert_eq!(normalise("oIl00-12345").as_deref(), Some("0110012345"));
        assert_eq!(normalise("ABCDE-FGHJ"), None);
        assert_eq!(normalise("ABCDE-FGHJKM"), None);
        assert_eq!(normalise("ABCDE-FGHJU"), None);
        assert_eq!(normalise("ABCDE_FGHJK"), None);
        assert_eq!(normalise("ÄBCDE-FGHJK"), None);
    }

    #[test]
    fn a_match_needs_every_character() {
        assert!(matches("ABCDEFGHJK", "abcde-fghjk"));
        assert!(!matches("ABCDEFGHJK", "ABCDE-FGHJM"));
        assert!(!matches("ABCDEFGHJK", "BBCDE-FGHJK"));
        assert!(!matches("ABCDEFGHJK", ""));
        assert!(!matches("", ""));
        assert_eq!(display("ABCDEFGHJK"), "ABCDE-FGHJK");
    }
}
