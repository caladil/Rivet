//! **What the name service is handed** — `ADR-047` §6.
//!
//! `rivet-dnsd` runs under its own identity and may not read the machine
//! document, which the confinement policy refuses every confined program. The
//! supervisor instead writes it a projection of that document: a valid
//! configuration document holding `version`, this machine's `hostname` and the
//! `[dns]` table, and nothing else. It is rendered by the document's own
//! serialiser and read back by the document's own parser, so there is one
//! grammar and one reader for the table (`ADR-011`).
//!
//! The file lives in the service's home beside its settings file, on
//! [`crate::settings::path`]'s footing: `<volume>/app/<name>` is root-owned at
//! `0755`, so the service cannot plant a link where root is about to write,
//! and it cannot rewrite the zone it serves.

/// The file's name, inside the service's home.
pub const FILE: &str = "dns.toml";

/// `<volume>/app/<name>/dns.toml` — **root-owned, readable by the service.**
#[must_use]
pub fn path(volume: &str, name: &crate::service::ServiceName) -> String {
    format!("{}/{FILE}", crate::service::app_dir(volume, name))
}

/// The mode the projection is written with — `0644`, root-owned.
///
/// The same as [`crate::settings::MODE`] and for its reason: the reader runs
/// as its own uid, and a file it cannot open is a machine that reports
/// delivery and serves nothing. The projection holds nothing the machine
/// document does not, and the document carries no secret (`ADR-038` §5).
pub const MODE: u32 = 0o644;

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn the_projection_sits_in_the_services_home_beside_its_settings() {
        let name = crate::service::ServiceName::new(crate::service::DNSD_SERVICE_NAME).unwrap();
        assert_eq!(path("/state", &name), "/state/app/rivet-dnsd/dns.toml");
        assert_eq!(
            std::path::Path::new(&path("/state", &name)).parent(),
            std::path::Path::new(&crate::settings::path("/state", &name)).parent(),
        );
    }
}
