//! **Home directories, and the uid range nothing on this machine occupies.**
//!
//! # The range is reserved and empty, and both halves are the point
//!
//! `ADR-008` §7.3.1 has two kinds of identity: *service* identities, derived
//! from an application's name and logged in as by nobody, and *human*
//! identities, which are deferred. **Rivet has none of the second kind.** There
//! is nothing to declare one with, nothing to authenticate one, and nothing that
//! would resolve one — `ADR-035`'s control plane authenticates a key, not a
//! Unix user, and an application runs as an identity derived from its own name
//! (`crate::service::app_identity`).
//!
//! So the range below is **reserved and unoccupied**. It is not dead: two of
//! this machine's other numbers are positioned against it and would be wrong if
//! it moved — `crate::etc::REPO_GID` is 999 so that it sits *below* the range,
//! and [`crate::service::APP_UID_BASE`] is 4000 so that it sits *above* it. Its
//! own documentation says why:
//!
//! > **1–999** is the conventional system range, which musl and every tool an
//! > operator has used before treat as "not a person"; and
//! > **1000–3999** is where human accounts conventionally start, which is where
//! > `ADR-008` §7.3's deferred human half will have to put them.
//!
//! **The ranges cannot overlap**, which is asserted at compile time rather than
//! assumed — two identities sharing a uid would be two answers to *who is this
//! process*, which is `ADR-011` in the place it is most expensive. A `const`
//! block catches a drift on every build, including a cross-compile by somebody
//! who never ran the tests.

/// The lowest uid the reserved account range covers — see the module header.
pub const ACCOUNT_UID_BASE: u32 = 1000;

/// The highest.
///
/// One below [`crate::service::APP_UID_BASE`], so the two ranges are adjacent
/// and disjoint. The `const` block below asserts it.
pub const ACCOUNT_UID_LAST: u32 = crate::service::APP_UID_BASE - 1;

/// The parent of every home directory, `/state/home`.
///
/// `ADR-008` §3.1 put home directories on `/state` rather than in an FHS
/// `/home` that does not exist. It is on `/state` and not `/system` because a
/// home directory is per-machine and must survive an update.
pub const HOME_PARENT: &str = "/state/home";

/// Root's home directory, `/state/home/root`.
///
/// The same layout as every other account's, which is what [`home`] asserts.
pub const ROOT_HOME: &str = "/state/home/root";

/// Where an account's home directory lives, under [`HOME_PARENT`].
///
/// `/state/home/<name>`, which is where `root`'s already is — one layout, so a
/// second kind of identity that ever needs a home does not get to invent a
/// second place to put it.
#[must_use]
pub fn home(name: &str) -> String {
    format!("{HOME_PARENT}/{name}")
}

// --- three invariants the COMPILER refuses, rather than a test that must be run
//
// **Moved out of `mod tests` on 2026-09-02**, on `clippy::assertions_on_constants`
// — which was right, and right in a way worth acting on rather than silencing.
//
// Each is a relationship between two `const`s. A test asserting one catches a
// drift only if somebody runs the tests; a `const` block catches it at every
// `cargo build`, including a cross-compile for a board by somebody who never
// ran `cargo test`. The failure mode they guard is `ADR-011` at its most
// expensive — two identities sharing a uid would be two answers to *who is this
// process* — and that is not a thing to find out on a machine.
const _: () = assert!(
    ACCOUNT_UID_LAST < crate::service::APP_UID_BASE,
    "an account uid could reach an application's, so one uid would name two things"
);
const _: () = assert!(
    ACCOUNT_UID_LAST + 1 == crate::service::APP_UID_BASE,
    "a gap between the account range and the application range is uids nothing can use"
);
const _: () = assert!(
    ACCOUNT_UID_BASE > 999,
    "an account inside the system range is one that tooling treats as `not a person`"
);
// **The other end**, which is the one this module is positioned against rather
// than the one it positions: `crate::etc::REPO_GID` is the last system gid, and
// it sits immediately below the range. A change to either number that closed the
// gap would put a group and an identity on one number.
const _: () = assert!(
    crate::etc::REPO_GID + 1 == ACCOUNT_UID_BASE,
    "the last system gid and the reserved range are no longer adjacent"
);

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// One layout with `root`'s, which is already `/state/home/root`.
    #[test]
    fn a_home_is_beside_roots() {
        assert_eq!(home("git"), "/state/home/git");
        assert_eq!(home("root"), ROOT_HOME);
    }

    /// **Nothing occupies it**, which is the fact this module exists to keep
    /// true rather than a gap waiting to be filled: every identity a Rivet has
    /// is uid 0, a system gid below the range, or an application above it.
    #[test]
    fn no_identity_this_machine_has_falls_inside_the_reserved_range() {
        for id in crate::etc::SYSTEM_IDENTITIES {
            assert!(id.uid < ACCOUNT_UID_BASE, "{}: {}", id.name, id.uid);
        }
        for g in crate::etc::SYSTEM_GROUPS {
            assert!(g.gid < ACCOUNT_UID_BASE, "{}: {}", g.name, g.gid);
        }
    }
}

