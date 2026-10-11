//! **What time this machine thinks it is locally** — one zone, one file.
//!
//! Ruled 2026-09-02: *the configuration document is the authority for this
//! machine's timezone, and `/etc` may project it if the C library needs it.*
//! `ADR-021` §9 is the ruling written down; this module is the type it needs.
//!
//! The absence it closes had been recorded three times and acted on none of
//! them: [`crate::etc`]'s table has carried *"what timezone this machine keeps
//! → the document (`ADR-021`) → no schema key yet"* since 2026-08-23,
//! `cmd/rivet-sysd`'s `inputs_from` repeats it, and `PLAN.md` §110.7 item 1
//! made it the first of the three things blocking work — *"the clock has no
//! time zone … `aura-renderer` runs on UTC and says so"*.
//!
//! # A name, not a set of rules
//!
//! This type holds an **IANA time zone identifier** — `America/Chicago`,
//! `Europe/Berlin`, `UTC`. It deliberately holds no offset and no
//! daylight-saving rule, because those are not facts about the operator's
//! intent: they are facts about a database that jurisdictions change, and
//! `ADR-011` gives them one authority — the tzfile on the image, which an A/B
//! update replaces.
//!
//! **A POSIX `TZ` string is the alternative and it was rejected.** musl accepts
//! `CST6CDT,M3.2.0/2,M11.1.0/2` with no database at all, which is genuinely
//! tempting for an image that would rather ship nothing. Two reasons it is not
//! what the schema takes:
//!
//! * **Nobody knows their POSIX string.** An operator knows they are in
//!   Chicago. Asking them to transcribe a transition rule is asking them to
//!   hand-maintain data.
//! * **It hard-codes rules that change.** The United States moved its DST
//!   boundaries in 2007 and has legislated about abolishing them since; a board
//!   carrying a literal rule would keep the old one for ever and be exactly one
//!   hour wrong for a few weeks a year, which is the failure mode nobody
//!   notices until a calendar is involved.
//!
//! The string form is still *reachable* — musl reads `TZ` if something sets it
//! — and nothing here sets it. One authority.
//!
//! # Why the grammar is enforced and not merely documented
//!
//! **This name becomes a path.** [`crate::etc`] projects `/etc/localtime` by
//! copying `<zoneinfo>/<name>` off the read-only system slot, so a `name`
//! containing `..` or a leading `/` would be a document choosing which file on
//! the machine gets copied into `/etc`. The grammar below refuses both by
//! construction — every component is drawn from a character class that contains
//! neither `.` nor `/` — and that is a containment property rather than
//! tidiness.
//!
//! The second reason is the one `ADR-034` §5.3 is about. A value that cannot be
//! a zone name would parse, validate, project **and change nothing**: musl
//! falls back to UTC when it cannot map a tzfile, silently, so the operator
//! would read their document, see a timezone, and have a UTC clock. Refusing at
//! parse time is the difference between a sentence naming the value and a
//! wrong clock nobody looks at.
//!
//! # What this type CANNOT check, stated rather than discovered
//!
//! **Whether the zone exists.** That is a question about the database this
//! particular image ships, and a document is written for machines that have not
//! been built yet — the same reason `crate::net::InterfaceName` does not refuse
//! an interface this machine does not have, and `rivet_config::volumes::Guid`
//! does not refuse a disk that is not attached (`ADR-007` §7.3). A well-formed
//! name for a zone the image has no file for is reported by the projection, on
//! the machine, at the moment it fails to copy it — loudly, and with the clock
//! left at UTC.

use core::fmt;

/// An IANA time zone identifier, validated.
///
/// See the module header for the grammar, for why it is enforced, and for what
/// it cannot check.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timezone(String);

/// What musl keeps when it has no tzfile to read.
///
/// `ADR-008` §5.3.1 read it out of the linked `libc.a` rather than out of
/// documentation: `__tz.lo` carries `UTC`, `GMT` and a `__utc` symbol, and
/// every path that fails to map a file ends at it. It is here so that a caller
/// can name the unconfigured state, and so that nothing has to reason about
/// musl's fallback from memory.
pub const UNCONFIGURED: &str = "UTC";

/// The longest a single `/`-separated component may be.
///
/// **Fourteen, and it is the tz database's own limit** rather than a number
/// chosen here: the project constrains names so that they remain distinct on
/// filesystems with short components, and its longest real components —
/// `Bahia_Banderas`, `ComodRivadavia`, `Port_of_Spain` — sit at or under it.
pub const MAX_COMPONENT_BYTES: usize = 14;

/// The longest a whole identifier may be.
///
/// The longest real name in the 2026c database is
/// `America/Argentina/Buenos_Aires` at 30 bytes; the cap is generous rather
/// than tight because a future database may add a longer one and this type
/// refusing a real zone would be worse than accepting a nonexistent one — the
/// projection reports that anyway. It is here at all so that nothing unbounded
/// reaches a fixed-size path buffer in a C library.
pub const MAX_BYTES: usize = 64;

impl Timezone {
    /// The rule a refusal quotes and the schema publishes. One definition.
    pub const RULE: &'static str = "an IANA time zone identifier such as \
                                    `America/Chicago`, `Europe/Berlin` or `UTC`: one or more \
                                    components separated by `/`, each 1 to 14 characters of \
                                    ASCII letters, digits, `_`, `+` or `-`, none of them \
                                    beginning with `-`, and at most 64 characters in all. It \
                                    is a NAME and not a path and not a POSIX TZ string: a \
                                    leading `/` and any `.` are refused, because this name is \
                                    used to read a file out of the image's zone database";

    /// Read a time zone identifier, or refuse it.
    ///
    /// **Case is preserved and not folded**, for `crate::hostname::Hostname`'s
    /// reason: `ADR-007` §3.1 requires a document to come back byte-identical,
    /// so a value that was silently rewritten on the way in is a defect. It is
    /// also load-bearing here in a way it is not for a hostname — a tzfile is
    /// looked up on a case-sensitive filesystem, so `america/chicago` is a
    /// different path from `America/Chicago` and only one of them exists.
    #[must_use]
    pub fn new(text: &str) -> Option<Self> {
        if text.is_empty() || text.len() > MAX_BYTES {
            return None;
        }
        // `split('/')` yields an empty component for a leading, trailing or
        // doubled separator, and the emptiness check below refuses all three —
        // which is what keeps `/etc/passwd`, `America//Chicago` and a trailing
        // slash out without any of them needing a rule of its own.
        for component in text.split('/') {
            if component.is_empty() || component.len() > MAX_COMPONENT_BYTES {
                return None;
            }
            if !component
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'+' || b == b'-')
            {
                return None;
            }
            // A component may CONTAIN a hyphen — `Port-au-Prince`, `GB-Eire` —
            // and may not begin with one. No real zone does, and a name that
            // did would reach a command line looking like an option.
            if component.starts_with('-') {
                return None;
            }
        }
        Some(Self(text.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Where this zone's tzfile is on a Rivet, given the image's database root.
    ///
    /// **The one place that path is composed**, so that the image builder, the
    /// projection and any refusal are naming one string rather than three
    /// (`ADR-011`). The root is an argument because it is a fact about the
    /// image's layout and this module is not where that is decided —
    /// `crate::etc::ZONEINFO_DIR` is.
    ///
    /// Safe to join by construction: [`Self::new`] has already refused a
    /// leading `/` and every `.`, so no value of this type can escape the root.
    #[must_use]
    pub fn tzfile_under(&self, root: &str) -> String {
        format!("{}/{}", root.trim_end_matches('/'), self.0)
    }
}

impl fmt::Display for Timezone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The four bytes every tzfile begins with — RFC 8536 §3.1.
///
/// **This is the check musl itself performs**, and it is the reason the
/// projection can refuse a file rather than shipping one that produces a UTC
/// clock: `__tz.lo` maps the file, compares the first four bytes to `TZif`, and
/// on a mismatch unmaps it and falls back to `__utc` **without saying
/// anything**. Copying a non-tzfile into `/etc/localtime` would therefore
/// succeed at every step and be silently wrong, which is `ADR-034` §5.3 exactly.
///
/// It is also what lets the image builder ship a *directory* honestly: a zone
/// database from a distribution carries `zone.tab`, `iso3166.tab`,
/// `tzdata.zi` and a `SECURITY` file beside the zones, and none of them is a
/// zone. The rule "ship every file that begins with `TZif`" reads the files
/// rather than trusting a list of names.
pub const TZIF_MAGIC: &[u8; 4] = b"TZif";

/// Whether these bytes are a tzfile musl would actually accept.
///
/// The length floor is musl's too: `__tz.lo` requires `map_size >= 44` before
/// it will look at the header, so a truncated file is refused there rather than
/// misread. Both halves are checked here so that a caller reporting a bad zone
/// file is reporting what musl would have done with it.
#[must_use]
pub fn is_tzfile(bytes: &[u8]) -> bool {
    bytes.len() >= MIN_TZFILE_BYTES && bytes.starts_with(TZIF_MAGIC)
}

/// The smallest thing musl will treat as a tzfile — see [`is_tzfile`].
pub const MIN_TZFILE_BYTES: usize = 44;

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    // ---- the grammar -------------------------------------------------------

    /// Names taken from the 2026c database the build container ships, read off
    /// `/usr/share/zoneinfo` rather than recalled — `AGENTS.md` §5.
    #[test]
    fn real_zone_names_are_accepted_as_typed() {
        for good in [
            "UTC",
            "America/Chicago",
            "Europe/Berlin",
            "America/Argentina/Buenos_Aires",
            "America/Port-au-Prince",
            "Etc/GMT+12",
            "GB-Eire",
            "Pacific/Kiritimati",
            "Asia/Ho_Chi_Minh",
        ] {
            let zone = Timezone::new(good).unwrap_or_else(|| panic!("{good:?} should be a zone"));
            assert_eq!(zone.as_str(), good, "case must be preserved");
        }
    }

    /// **The containment property, asserted rather than assumed.** Every one of
    /// these would be a document choosing a file to copy into `/etc`.
    #[test]
    fn a_name_that_could_be_a_path_is_refused() {
        for bad in [
            "/etc/passwd",
            "../../etc/shadow",
            "America/../../etc/passwd",
            ".",
            "..",
            "./America/Chicago",
            "America/Chicago/",
            "America//Chicago",
            "/",
        ] {
            assert_eq!(Timezone::new(bad), None, "{bad:?} was accepted");
        }
    }

    #[test]
    fn the_rest_of_the_grammar_is_enforced() {
        for bad in [
            "",
            "America/Chicago ",
            "America Chicago",
            "-leading",
            "America/-leading",
            "emoji-\u{1f600}",
            "CST6CDT,M3.2.0/2,M11.1.0/2",
            "America/ThisComponentIsFarTooLong",
        ] {
            assert_eq!(Timezone::new(bad), None, "{bad:?} was accepted");
        }
        assert_eq!(Timezone::new(&"a".repeat(MAX_BYTES + 1)), None);
        // A long name made of legal components is accepted right up to the cap.
        let long = ["abcdefghijklmn"; 4].join("/");
        assert_eq!(long.len(), 59);
        assert!(Timezone::new(&long).is_some());
    }

    /// A POSIX `TZ` string is the thing this key is NOT, and the refusal has to
    /// be visible in a test or the module header is an opinion.
    #[test]
    fn a_posix_tz_string_is_not_a_zone_name() {
        assert_eq!(Timezone::new("CST6CDT,M3.2.0/2,M11.1.0/2"), None);
        // `CST6CDT` alone IS a real zone file, so it is accepted — the comma
        // form is what is refused, and the two are one character apart.
        assert!(Timezone::new("CST6CDT").is_some());
    }

    // ---- the path ----------------------------------------------------------

    #[test]
    fn the_tzfile_path_is_composed_in_exactly_one_place() {
        let zone = Timezone::new("America/Chicago").expect("a real zone");
        assert_eq!(
            zone.tzfile_under("/system/share/zoneinfo"),
            "/system/share/zoneinfo/America/Chicago"
        );
        // A trailing slash on the root does not double.
        assert_eq!(
            zone.tzfile_under("/system/share/zoneinfo/"),
            "/system/share/zoneinfo/America/Chicago"
        );
    }

    /// The containment property again, at the other end: no accepted value can
    /// produce a path outside the root.
    #[test]
    fn no_accepted_name_escapes_the_database_root() {
        for name in ["UTC", "America/Chicago", "America/Argentina/Buenos_Aires"] {
            let path = Timezone::new(name)
                .expect("a real zone")
                .tzfile_under("/system/share/zoneinfo");
            assert!(path.starts_with("/system/share/zoneinfo/"), "{path}");
            assert!(!path.contains(".."), "{path}");
        }
    }

    // ---- the magic ---------------------------------------------------------

    #[test]
    fn a_tzfile_is_recognised_by_the_bytes_musl_checks() {
        let mut good = Vec::from(*TZIF_MAGIC);
        good.extend(std::iter::repeat_n(0u8, MIN_TZFILE_BYTES - 4));
        assert!(is_tzfile(&good));

        // Truncated: musl refuses below 44 bytes before it looks at the header.
        assert!(!is_tzfile(&good[..MIN_TZFILE_BYTES - 1]));
        // The tab files that sit beside the zones in a distribution's database.
        assert!(!is_tzfile(b"# tz zone descriptions\nUS\tAmerica/Chicago\n\n\n\n\n\n\n\n\n"));
        assert!(!is_tzfile(&[]));
    }

    /// The published rule describes what is actually enforced — asserted rather
    /// than trusted, as `Hostname::RULE` is.
    #[test]
    fn the_published_rule_describes_what_is_actually_enforced() {
        assert!(Timezone::RULE.contains(&MAX_COMPONENT_BYTES.to_string()));
        assert!(Timezone::RULE.contains(&MAX_BYTES.to_string()));
        assert!(Timezone::RULE.contains("America/Chicago"));
        assert!(Timezone::RULE.contains("NAME"));
    }
}
