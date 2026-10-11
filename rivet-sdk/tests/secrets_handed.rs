//! **A secret is read from the directory the application was HANDED** —
//! `ADR-038` §5, `.rulings/` R-169.
//!
//! # Why this is its own test binary and not another case in `agreement.rs`
//!
//! `std::env::set_var` races any other thread reading the environment, which
//! is why it is `unsafe` in edition 2024 — `tests/settings_handed.rs` states
//! the same reason for the same shape of test. A binary with exactly one test
//! function has no other thread to race, so the hazard is removed by
//! construction.
//!
//! # What it is for
//!
//! `rivet_sdk::secrets` reads through [`rivet_sdk::app::data`], the same
//! `RIVET_APP_DATA` every application already has — unlike settings, nothing
//! new is handed over (see the module documentation on `rivet_sdk::secrets`
//! for why `install-secret`'s write was never moved out of it). So the thing
//! worth proving end to end is that a file dropped where `install-secret`
//! would have put it — the directory named by `RIVET_APP_DATA`, under the
//! name given to `install-secret` — is exactly what this call reads back, and
//! that nothing here ever prints the value it read.

// A test asserts; `unwrap`, `expect` and `panic!` ARE the assertion.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// See `tests/settings_handed.rs`'s module note: two `std::env` calls, each
// with its own SAFETY comment, in a binary proven to hold no other thread
// that could race them.
#![allow(unsafe_code)]

/// A value that is obviously a placeholder and not a real credential —
/// `AGENTS.md` §8.3, zero-scrub: nothing realistic-looking goes in a tracked
/// fixture, hand-composed or otherwise.
const FIXTURE_SECRET: &str = "PLACEHOLDER-NOT-A-REAL-SECRET";

#[test]
fn a_secret_where_install_secret_would_have_put_it_is_what_this_call_reads() {
    let directory = std::env::temp_dir().join("rivet-sdk-handed-secrets-test");
    std::fs::create_dir_all(&directory).expect("a scratch directory");

    // SAFETY: this binary holds one test function, so no other thread of this
    // process can be reading the environment while it is changed.
    unsafe { std::env::remove_var(rivet_sdk::app::APP_DATA_ENV) };

    // **Nothing handed over is no data directory, so no secret is ever
    // found** — the state of every process a supervisor did not start.
    assert!(rivet_sdk::secrets::get("relay.env").is_none());
    assert!(rivet_sdk::secrets::get_bytes("relay.env").is_none());
    assert!(rivet_sdk::secrets::path("relay.env").is_none());

    // SAFETY: as above.
    unsafe { std::env::set_var(rivet_sdk::app::APP_DATA_ENV, &directory) };

    // Nothing installed yet: absent, not an error.
    assert_eq!(rivet_sdk::secrets::get("relay.env"), None);

    // `install-secret <application> relay.env --hex <hex>` writes the DECODED
    // bytes directly into this directory under the given name — this is that
    // write, without the plan and the confirmation gate around it, because
    // this test is about the reading half.
    std::fs::write(directory.join("relay.env"), FIXTURE_SECRET.as_bytes())
        .expect("the fixture writes");

    assert_eq!(rivet_sdk::secrets::get("relay.env").as_deref(), Some(FIXTURE_SECRET));
    assert_eq!(
        rivet_sdk::secrets::get_bytes("relay.env").as_deref(),
        Some(FIXTURE_SECRET.as_bytes())
    );
    assert_eq!(
        rivet_sdk::secrets::path("relay.env"),
        Some(directory.join("relay.env"))
    );

    // A name nothing installed is absent, not an empty string.
    assert_eq!(rivet_sdk::secrets::get("write-token"), None);

    // A name that could not be one `install-secret` would have used is
    // refused before a path is even composed — never normalised into
    // something that happens to still land inside `directory`.
    assert_eq!(rivet_sdk::secrets::get("../outside"), None);
    assert_eq!(rivet_sdk::secrets::path("../outside"), None);

    // SAFETY: as above.
    unsafe { std::env::remove_var(rivet_sdk::app::APP_DATA_ENV) };
    let _ = std::fs::remove_dir_all(&directory);
}
