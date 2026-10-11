//! **The applications a card carries** — `ADR-050` §4, ruled 2026-09-07.
//!
//! # The sentence this exists to make true
//!
//! `SPEC-05` §1 is the maintainer's, in substance: once the device has booted,
//! `fleetd` is up and running in the way `mcpd` is (`.rulings/` R-303). It could
//! not be true of any card this project could build, and the reason was a
//! distinction nobody had written down: **`rivet-mcpd` is in the system image
//! and `rivet-fleetd` is an application.** `ADR-049` put the per-machine
//! *material* on the card and left the *programs* absent.
//!
//! This module is the naming half of the other side. A card may carry signed
//! `.rvt` payloads; PID 1 lands them in `/state`; the supervisor asks
//! `cmd/rivet-deploy` to install each one, and `ADR-049`'s staged credentials
//! are delivered by the discovery pass that install triggers.
//!
//! # It performs no I/O and decides no policy
//!
//! `ADR-014` §3. What is here is the set of **names** three programs have to
//! agree on — `cmd/rivet-image` writes them, `cmd/rivet-initd` reads them, and
//! `cmd/rivet-sysd` consumes them — and `ADR-011` puts a fact like that in one
//! place. Two spellings of `app-<product>.rvt` in two crates is a card whose
//! payloads are written under one name and looked for under another, which
//! fails as *nothing happened* rather than as an error.
//!
//! # The shape, and why the record directory outlives the payload
//!
//! ```text
//!   boot partition        /state/carried/<product>/        what it means
//!   app-<product>.rvt  -> payload.rvt                      carried, not installed yet
//!                         (directory, empty)               carried, and dealt with
//! ```
//!
//! **The directory is never removed and the payload is.** That pairing is what
//! makes the whole thing idempotent across boots without a second kind of state:
//!
//! * PID 1 copies a payload off the card only when the record directory is
//!   **absent**, so the second boot of a card copies nothing.
//! * The supervisor removes only the **payload** when its install has finished,
//!   so the directory that stops PID 1 re-copying survives the install.
//!
//! The alternative — destroying the card's copy the way `ADR-049`'s staging
//! bundle is destroyed — was rejected because the reason staging is destroyed
//! does not apply here. A staging bundle holds `server.key` and an operator
//! seed and the card is the whole boundary (`ADR-020` §11); a `.rvt` is a
//! **signed, public artifact** whose integrity is protected by a signature
//! rather than by secrecy. Leaving it on the card costs a few megabytes of a
//! 256 MiB partition and buys a card that still says what it was built to carry
//! when somebody puts it in a reader.
//!
//! # One directory per payload, and no directory walking anywhere
//!
//! `ADR-049` §5 chose one file per application over an unpacked tree because
//! the supervisor's `Kernel` port has no recursive walker in it and is
//! documented as not wanting one. The same constraint applies here and gets the
//! same answer one level up: the record is a **directory**, so the supervisor
//! enumerates it with `list_dir`, which answers directories only and whose
//! filter is a security property rather than a convenience.

use crate::service::ServiceName;

/// Where a carried payload comes to rest, between PID 1 and the installer.
pub const CARRIED_DIR: &str = crate::state::CARRIED;

/// What every name on the boot partition begins with.
///
/// **A prefix rather than a directory**, because `cmd/rivet-image`'s boot
/// partition is a flat list of `(name, bytes)` and its `check_name` refuses a
/// `/` — the same constraint that made `tools/card/build.sh` merge the display
/// overlay into the device tree rather than ship an `overlays/` directory.
pub const PREFIX: &str = "app-";

/// What every name on the boot partition ends with. `ADR-029`'s extension.
pub const SUFFIX: &str = ".rvt";

/// The one file inside a record directory, while there is one.
pub const PAYLOAD_FILE: &str = "payload.rvt";

/// The mode the record directories are created with.
///
/// [`crate::staging::DIR_MODE`]'s value, and the same reasoning one step
/// weaker: nothing in here is a secret, and a payload an unprivileged process
/// could replace between the courier and the installer would be a signature
/// check performed on bytes nobody chose. `cmd/rivet-deploy` runs as uid 0 and
/// reads it either way.
pub const DIR_MODE: u32 = 0o700;

/// The mode a payload is written with.
pub const FILE_MODE: u32 = 0o600;

/// The largest payload PID 1 will carry off a card, in bytes.
///
/// **A limit, because PID 1 reads this into memory**, and PID 1 is the process
/// whose failure is the machine (`ADR-017` §3). Thirty-two mebibytes is twice
/// `rivet_core::package::application::MAX_PROGRAM_BYTES` and leaves room for the
/// assets beside it, while staying an eighth of the boot partition — so a
/// truncated or hostile card cannot make the allocation the reason a board does
/// not boot.
pub const MAX_PAYLOAD_BYTES: usize = 32 * 1024 * 1024;

/// What a payload for `product` is called on the boot partition.
///
/// **The product name and not the source file's name.** A `.rvt` on a
/// workstation is called whatever `cargo rivet package` or an operator called
/// it, and `ADR-027` §5 already refuses to read a fact out of a file name —
/// `cmd/rivet-image` verifies the payload at build time and takes the product
/// out of the **signed header**, which is the authority for what a machine
/// will call it (`ADR-015` §7).
#[must_use]
pub fn card_file(product: &str) -> String {
    format!("{PREFIX}{product}{SUFFIX}")
}

/// The product a boot-partition name carries, or `None` if it is not one.
///
/// [`card_file`] backwards, and the round trip is what keeps the writer and the
/// two readers from drifting. `None` for anything that is not `app-<name>.rvt`
/// with a `<name>` a machine would accept as a service name — PID 1 skips those
/// silently, because a boot partition holds `config.txt`, a kernel and a device
/// tree and none of them is a payload.
#[must_use]
pub fn product_of(file_name: &str) -> Option<String> {
    let rest = file_name.strip_prefix(PREFIX)?;
    let product = rest.strip_suffix(SUFFIX)?;
    // **Checked rather than assumed.** The name becomes a directory the
    // supervisor joins paths onto, so a `..` or a `/` reaching this far would
    // be the escape `rivet_core::staging::check_path` exists to refuse one
    // layer down. `ServiceName` already forbids both, and it is the same rule
    // the installer will apply to the header when it unpacks.
    Some(ServiceName::new(product)?.as_str().to_string())
}

/// The directory that records this product was carried.
///
/// **It outlives the payload**, deliberately — see the module header. Its
/// presence is the whole of what stops PID 1 copying the same payload off the
/// card on every boot for ever, which is `cmd/rivet-sysd`'s
/// `deploy_request` hazard arriving by a slower route: each iteration would
/// write the payload to `/state` and install it again.
#[must_use]
pub fn record_dir(product: &str) -> String {
    format!("{CARRIED_DIR}/{product}")
}

/// The payload itself, while it is waiting to be installed.
#[must_use]
pub fn payload_path(product: &str) -> String {
    format!("{}/{PAYLOAD_FILE}", record_dir(product))
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// The writer and the readers agree, and that is what makes
    /// three crates naming one file safe.
    #[test]
    fn a_card_name_round_trips_through_the_product_it_names() {
        for product in ["rivet-fleetd", "rivet-proxyd", "aura-renderer", "a"] {
            let name = card_file(product);
            assert_eq!(name, format!("app-{product}.rvt"));
            assert_eq!(product_of(&name).as_deref(), Some(product));
        }
    }

    /// Everything else on a boot partition is not a payload, and PID 1 has to
    /// be able to tell without a table of exceptions.
    #[test]
    fn nothing_else_on_a_boot_partition_reads_as_a_payload() {
        for other in [
            "config.txt",
            "tryboot.txt",
            "autoboot.txt",
            "cmdline.txt",
            "kernel8.img",
            "bcm2711-rpi-4-b.dtb",
            "start4.elf",
            "fixup4.dat",
            "rivet.toml",
            "device.seed",
            "device.crt",
            "peers",
            "staged.txt",
            // The prefix without the suffix, and the suffix without the prefix.
            "app-rivet-fleetd",
            "rivet-fleetd.rvt",
            "app-.rvt",
        ] {
            assert_eq!(product_of(other), None, "{other} read as a payload");
        }
    }

    /// **The escape this refuses, at the only place it could be refused.** The
    /// supervisor joins this name onto a directory it owns and PID 1 writes
    /// into it as root, so a name carrying a separator or a parent component
    /// would put a file wherever it liked.
    #[test]
    fn a_name_that_would_escape_the_carried_directory_is_not_a_product() {
        for hostile in [
            "app-../../etc.rvt",
            "app-..rvt",
            "app-/absolute.rvt",
            "app-a/b.rvt",
            "app- space.rvt",
            "app-x\0y.rvt",
        ] {
            assert_eq!(product_of(hostile), None, "{hostile} was accepted");
        }
    }

    /// **A product name may carry capitals, and FAT cannot tell two of those
    /// apart.** `rivet_core::service::ServiceName` accepts `Foo` and `foo` as
    /// two names, and this module answers both faithfully — but the boot
    /// partition is FAT, where the two file names are one file. The refusal
    /// therefore belongs where the card is composed and is
    /// `cmd/rivet-image`'s: `two_payloads_whose_names_differ_only_in_case_are_refused`.
    /// Asserted here so that the hazard is written down beside the function
    /// that produces it rather than only beside the guard.
    #[test]
    fn two_products_differing_only_in_case_are_two_products_here() {
        assert_ne!(card_file("Foo"), card_file("foo"));
        assert_eq!(product_of("app-Foo.rvt").as_deref(), Some("Foo"));
        assert_eq!(
            card_file("Foo").to_ascii_lowercase(),
            card_file("foo"),
            "if this ever differs, the FAT collision is gone and the guard in \
             cmd/rivet-image can be reconsidered"
        );
    }

    /// The two paths are one path apart, so nothing can look for a payload in a
    /// directory that is not its record.
    #[test]
    fn the_payload_lives_inside_the_record_that_outlives_it() {
        let dir = record_dir("rivet-fleetd");
        assert_eq!(dir, "/state/carried/rivet-fleetd");
        assert_eq!(
            payload_path("rivet-fleetd"),
            format!("{dir}/{PAYLOAD_FILE}")
        );
        assert!(dir.starts_with(&format!("{CARRIED_DIR}/")));
    }

    /// It is on the filesystem an update never touches, and it is not inside
    /// `/state/staging` — two lifetimes, two directories, exactly as `ADR-049`
    /// separated staging from `/state/secrets`.
    #[test]
    fn carried_material_is_its_own_directory_on_the_writable_filesystem() {
        assert_eq!(CARRIED_DIR, "/state/carried");
        assert!(CARRIED_DIR.starts_with(&format!("{}/", crate::state::STATE_MOUNT_POINT)));
        assert_ne!(CARRIED_DIR, crate::state::STAGING);
        assert!(!CARRIED_DIR.starts_with(&format!("{}/", crate::state::STAGING)));
        assert!(!crate::state::STAGING.starts_with(&format!("{CARRIED_DIR}/")));
    }

    /// Nobody but root reads either, which is what keeps a payload from being
    /// swapped between the courier and the verifier.
    #[test]
    fn nothing_carried_is_readable_by_an_unprivileged_account() {
        assert_eq!(DIR_MODE & 0o077, 0);
        assert_eq!(FILE_MODE & 0o077, 0);
    }
}
