//! **The settings file is read at the path the application was HANDED** —
//! `ADR-038` §8.1, 2026-09-04.
//!
//! # Why this is its own test binary and not another case in `agreement.rs`
//!
//! It sets an environment variable, and `std::env::set_var` races any other
//! thread that reads the environment — which is why it is `unsafe` in edition
//! 2024 and why `cmd/rivet-sysd`'s `kernel` module says so where it declined to
//! use it. A binary with **exactly one** test function has no other thread to
//! race, so the hazard is removed by construction rather than by a comment.
//!
//! # What it is for
//!
//! Until 2026-09-04 the two ends agreed by composition: the supervisor wrote
//! `<data>/settings` and the SDK joined the same filename onto `RIVET_APP_DATA`.
//! The file moved out of that directory so that the platform stops writing by
//! path below an unprivileged uid's own tree, and the SDK now reads a path it is
//! told. **An SDK that had gone on composing one would compile, pass every
//! agreement test, and read a file nothing writes** — which is the failure this
//! covers and the one nothing else can see.

// A test asserts; `unwrap`, `expect` and `panic!` ARE the assertion.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// **`-W unsafe-code` is on for the whole workspace and this file needs two.**
// Allowed here and nowhere else in this crate — `src/lib.rs` keeps
// `#![deny(unsafe_code)]`, so the SDK itself still cannot contain any. The two
// blocks are `std::env` calls, which edition 2024 made `unsafe` because they
// race a concurrent reader; the module documentation above says why this
// binary has no concurrent reader, and each carries its own SAFETY note.
#![allow(unsafe_code)]

#[test]
fn the_handed_path_is_what_is_read_and_an_unhanded_one_is_no_settings() {
    let name = rivet_core::service::ServiceName::new("aura-renderer").expect("a name");
    let volume = rivet_core::service::BUILT_IN_VOLUME;

    // SAFETY: this binary holds one test function, so no other thread of this
    // process can be reading the environment while it is changed.
    unsafe { std::env::remove_var(rivet_sdk::app::APP_SETTINGS_ENV) };

    // **Nothing handed over is no settings and never an error** — `ADR-036`
    // §8.5.2, and it is the state of every process a supervisor did not start.
    assert!(rivet_sdk::app::settings_file().is_err());
    assert!(rivet_sdk::settings::all().is_empty());
    assert_eq!(rivet_sdk::settings::get("brightness"), None);

    // Now hand one over, written by the platform's own renderer rather than
    // composed here — `AGENTS.md` §4: a fixture written by the reader's author
    // proves the reader agrees with itself.
    let (text, rejected) = rivet_core::settings::render(
        &["brightness".to_string()],
        &[("brightness".to_string(), "60".to_string())],
    );
    assert!(rejected.is_empty(), "{rejected:?}");

    let at = std::env::temp_dir().join("rivet-sdk-handed-settings");
    std::fs::write(&at, text.as_bytes()).expect("a temporary file");
    // SAFETY: as above.
    unsafe { std::env::set_var(rivet_sdk::app::APP_SETTINGS_ENV, &at) };

    assert_eq!(rivet_sdk::settings::get("brightness").as_deref(), Some("60"));
    assert_eq!(rivet_sdk::settings::get_parsed::<u8>("brightness"), Some(60));

    // **And composing the old path would NOT have found it.** This is the whole
    // point of the move: the file is outside the directory the application owns.
    let owned = rivet_core::service::data_dir(volume, &name);
    assert!(
        !rivet_core::settings::path(volume, &name).starts_with(&owned),
        "the settings file is still below {owned}, which the application owns at 0700"
    );

    // SAFETY: as above.
    unsafe { std::env::remove_var(rivet_sdk::app::APP_SETTINGS_ENV) };
    let _ = std::fs::remove_file(&at);
}
