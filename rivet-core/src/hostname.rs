//! **What this machine calls itself** — one name, three readers.
//!
//! Ruled 2026-08-26. The absence this closes had been recorded twice and acted
//! on neither time: `crate::etc`'s table has carried *"the machine's hostname →
//! `/etc/hosts` → blocked on no `hostname` key"* since 2026-08-23, and
//! `PLAN.md` §23.2 asserted that *"`ADR-007`'s document has one to give"* —
//! which was **wrong when it was written**, as `PLAN.md` §39.6 records.
//!
//! The board found the cost concretely: `/proc/sys/kernel/hostname` on the test
//! machine reads the seven bytes `(none)`, which is Linux's default when nothing
//! ever calls `sethostname(2)`. `<hostname>.local` would have been
//! `(none).local`.
//!
//! # One name, three readers
//!
//! | Reader | What it does with it |
//! | :-- | :-- |
//! | `sethostname(2)` | `uname -n`, and what a program asking the kernel gets |
//! | [`crate::etc`]'s `/etc/hosts` | so the machine can resolve its own name without a network |
//! | `cmd/rivet-mdnsd` | `<hostname>.local`, `PLAN.md` §23 |
//!
//! `ADR-011`: one authority. The failure the ruling refused was two names for
//! one machine — `uname` saying one thing and the network answering to another,
//! which is the disagreement nobody notices until they are debugging something
//! else.
//!
//! # Absent is a derived name, not silence
//!
//! **A machine nobody has named is still a machine somebody has to reach.** A
//! shelf of unconfigured boards all answering to `(none).local` is the state
//! mDNS was wanted to fix.
//!
//! So an absent key means [`derived`], and [`Hostname::DERIVED_PREFIX`] is what
//! it looks like.
//!
//! **A derived name must be distinct or it is worse than none.** Two machines
//! answering to one `.local` name is a network that has learned a lie, which is
//! `PLAN.md` §23.2's fourth bullet arriving before the service exists. So the
//! derivation takes a **per-machine** value and this module will not invent one:
//! [`derived`] takes the serial as an argument and the caller is the thing that
//! knows how to read it. A machine that cannot say which one it is **keeps
//! `(none)` and says so**, which is `AGENTS.md` §5 — refuse rather than guess —
//! applied to a name.
//!
//! # The grammar is RFC 1123's, and it is narrower than it looks
//!
//! A label is 1–63 characters of ASCII letters, digits and hyphens; it may not
//! begin or end with a hyphen. **Rivet takes the label and not the dotted
//! name**: `<hostname>.local` is composed by the responder, and a `hostname`
//! containing a dot would produce `a.b.local`, which is a different name in a
//! different zone from the one the operator meant.

use core::fmt;

/// What this machine calls itself.
///
/// A validated label — see the module header for the grammar and for why it is
/// a label rather than a dotted name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hostname(String);

/// What the kernel reports when nothing has ever called `sethostname(2)`.
///
/// **Read off the test board on 2026-08-26**, not out of documentation
/// (`AGENTS.md` §5): `/proc/sys/kernel/hostname` held exactly these seven
/// bytes. It is here so that a caller can recognise the unset state rather than
/// treat it as a name, and so that nothing ever tries to announce it.
pub const KERNEL_DEFAULT: &str = "(none)";

/// The longest a label may be — RFC 1123.
pub const MAX_BYTES: usize = 63;

impl Hostname {
    /// What a derived name begins with.
    ///
    /// **The project's own name, so that a machine nobody has named still says
    /// what it is.** An operator meeting `rivet-a1b2c3d4` on their network
    /// learns two things at once: what it is, and that nobody has named it yet.
    pub const DERIVED_PREFIX: &'static str = "rivet-";

    /// How many hexadecimal characters of the derivation reach the name.
    ///
    /// Eight — 32 bits. Enough that a household fleet will not collide, short
    /// enough to read over the phone, and it is the same reasoning
    /// `rivet-fleet`'s identifier uses at twice the width for a value that must
    /// survive a much larger register.
    pub const DERIVED_HEX: usize = 8;

    /// The rule a refusal quotes and the schema publishes. One definition.
    pub const RULE: &'static str = "1 to 63 characters, ASCII letters, digits and hyphens only, \
                                    not beginning or ending with a hyphen. It is a LABEL and not \
                                    a dotted name: mdnsd composes <hostname>.local, so a dot here \
                                    would announce a name in a different zone from the one you \
                                    meant";

    /// Read a hostname, or refuse it.
    ///
    /// **Case is preserved and not folded.** DNS comparison is
    /// case-insensitive, so folding would cost nothing functionally and would
    /// silently rewrite what the operator typed — and `ADR-007` §3.1's
    /// byte-identical round trip means a document that came back different from
    /// what went in is a document this project treats as a defect.
    #[must_use]
    pub fn new(text: &str) -> Option<Self> {
        if text.is_empty() || text.len() > MAX_BYTES {
            return None;
        }
        if !text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return None;
        }
        if text.starts_with('-') || text.ends_with('-') {
            return None;
        }
        // `(none)` is refused as a *typed* name even though the grammar would
        // reject it anyway on the parentheses: the point is that the unset
        // state is not a name, and a reader looking for that rule should find
        // it stated rather than inferred from a character class.
        if text == KERNEL_DEFAULT {
            return None;
        }
        Some(Self(text.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Hostname {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The name a machine gives itself when the document names none.
///
/// `serial` is **whatever this machine can say about itself that no other
/// machine would say** — on a Raspberry Pi that is `/proc/cpuinfo`'s `Serial`
/// line. Reading it is the caller's, because it is a platform question and this
/// crate performs no I/O (`ADR-014` §3).
///
/// **The serial is hashed rather than used directly**, and it is worth one
/// sentence: the name ends up on a multicast group that every device on the
/// link can hear, and a hardware serial announced to the LAN is a fact about
/// the machine that nothing needed. BLAKE3 is already linked wherever this is
/// called from, so it costs nothing to not leak it.
///
/// `None` when `serial` is empty or is itself the unset marker — a machine that
/// cannot distinguish itself must keep `(none)` rather than claim a name that
/// might collide. See the module header.
#[must_use]
pub fn derived(serial_digest: &[u8]) -> Option<Hostname> {
    if serial_digest.is_empty() {
        return None;
    }
    let mut name = String::from(Hostname::DERIVED_PREFIX);
    for byte in serial_digest
        .iter()
        .take(Hostname::DERIVED_HEX.div_ceil(2))
    {
        name.push_str(&format!("{byte:02x}"));
    }
    name.truncate(Hostname::DERIVED_PREFIX.len() + Hostname::DERIVED_HEX);
    Hostname::new(&name)
}

/// Whether a machine has said anything about which one it is.
///
/// The predicate a caller applies to a raw serial **before** hashing it, so
/// that an unreadable `/proc/cpuinfo` and a serial of `(none)` are one case.
#[must_use]
pub fn distinguishes_a_machine(serial: &str) -> bool {
    let serial = serial.trim();
    !serial.is_empty() && serial != KERNEL_DEFAULT && serial.bytes().any(|b| b != b'0')
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    // ---- the grammar -------------------------------------------------------

    #[test]
    fn an_ordinary_name_is_accepted_as_typed() {
        for good in ["kitchen-pi", "a", "photo-index-01", "Cedar", "x9"] {
            let name = Hostname::new(good).unwrap_or_else(|| panic!("{good:?} should be a name"));
            // Case is preserved: see `Hostname::new`.
            assert_eq!(name.as_str(), good);
        }
    }

    /// **A label, not a dotted name.** A `hostname` with a dot in it would make
    /// `mdnsd` announce `a.b.local` — a different name in a different zone from
    /// the one the operator meant.
    #[test]
    fn a_dotted_name_is_refused_because_mdnsd_would_announce_the_wrong_zone() {
        assert_eq!(Hostname::new("kitchen.pi"), None);
        assert_eq!(Hostname::new("kitchen-pi.local"), None);
    }

    #[test]
    fn the_rest_of_rfc_1123_is_enforced() {
        for bad in [
            "",
            "-leading",
            "trailing-",
            "under_score",
            "spaces here",
            "emoji-\u{1f600}",
            "colon:name",
        ] {
            assert_eq!(Hostname::new(bad), None, "{bad:?}");
        }
        assert_eq!(Hostname::new(&"a".repeat(MAX_BYTES + 1)), None);
        assert!(Hostname::new(&"a".repeat(MAX_BYTES)).is_some());
    }

    /// **The unset state is not a name.** Refused explicitly rather than by
    /// accident of the character class, so that the rule is findable.
    #[test]
    fn the_kernels_unset_marker_is_not_a_name() {
        assert_eq!(Hostname::new(KERNEL_DEFAULT), None);
    }

    // ---- the derived default -----------------------------------------------

    /// The shape an operator meets on their network before they have named
    /// anything: it says what the machine is, and that nobody has named it.
    #[test]
    fn a_derived_name_says_what_it_is_and_that_nobody_named_it() {
        let name = derived(&[0xa1, 0xb2, 0xc3, 0xd4, 0xff]).expect("a derivable machine");
        assert_eq!(name.as_str(), "rivet-a1b2c3d4");
        assert!(name.as_str().starts_with(Hostname::DERIVED_PREFIX));
    }

    /// A derived name is a name, so everything downstream may assume the
    /// grammar without re-checking it.
    #[test]
    fn a_derived_name_is_always_a_valid_one() {
        for byte in 0u8..=255 {
            let digest = [byte; 32];
            let name = derived(&digest).expect("any digest derives a name");
            assert_eq!(Hostname::new(name.as_str()).as_ref(), Some(&name));
        }
    }

    /// **Stable, because a name that changed on every boot would be worse than
    /// none** — mDNS caches, and so does every operator's shell history.
    #[test]
    fn the_same_machine_derives_the_same_name_every_time() {
        let digest = [0x11, 0x22, 0x33, 0x44];
        assert_eq!(derived(&digest), derived(&digest));
    }

    /// **Distinct, because two machines answering to one `.local` name is a
    /// network that has learned a lie.**
    #[test]
    fn two_machines_do_not_derive_one_name() {
        assert_ne!(derived(&[1, 2, 3, 4]), derived(&[1, 2, 3, 5]));
    }

    /// A machine that cannot say which one it is keeps `(none)` rather than
    /// claiming a name that might collide.
    #[test]
    fn a_machine_that_cannot_distinguish_itself_derives_nothing() {
        assert_eq!(derived(&[]), None);
        for indistinct in ["", "   ", KERNEL_DEFAULT, "0000000000000000"] {
            assert!(
                !distinguishes_a_machine(indistinct),
                "{indistinct:?} was taken as a machine identity"
            );
        }
        assert!(distinguishes_a_machine("10000000a1b2c3d4"));
    }

    /// The published rule is the grammar the code enforces — asserted rather
    /// than trusted, as `MODE_WORDS` is against `Mode::ALL`.
    #[test]
    fn the_published_rule_describes_what_is_actually_enforced() {
        assert!(Hostname::RULE.contains(&MAX_BYTES.to_string()));
        assert!(Hostname::RULE.contains("hyphen"));
        assert!(Hostname::RULE.contains("LABEL"));
    }
}
