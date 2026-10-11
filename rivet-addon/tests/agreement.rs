//! **`rivetos-addon` re-declares; this proves it still agrees** — the same
//! contract `rivet-sdk/tests/agreement.rs` carries, for the one constant this
//! crate restates.
//!
//! A host application and an addon are both foreign programs and must not link
//! the operating system (`ADR-036` §5), so `rivetos_addon` cannot import
//! `rivet_core::service::ADDONS_ENV` — it restates it. The cost of restating is
//! drift, and this file is the whole of what buys the right to do it.
//!
//! `rivetos-core` is a **dev-dependency** of `rivetos-addon` and appears
//! nowhere else. Dev-dependencies are not linked into consumers, so an
//! application that depends on `rivetos-addon` never sees `rivet_core`; the
//! edge exists only here.
//!
//! # Why a drifted spelling here would be silent
//!
//! That is the point. `rivet_core::service::addon_env` returns `None` when
//! nothing joined, and `rivetos_addon::host::Loaded::open` treats an **absent**
//! variable as a machine that names nothing and falls back to the old derived
//! name. A misspelling on either side is therefore indistinguishable from a
//! board running an older image: the host would keep working, would keep
//! opening exactly one addon per class, and would silently be unable to see a
//! second one — which is the entire capability this channel was built for.

// A test asserts; `unwrap`, `expect` and `panic!` ARE the assertion. The same
// allowance `rivet-core`'s own test modules carry, spelled at file scope
// because an integration test is not a `#[cfg(test)] mod`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(unix)]

use std::path::PathBuf;

/// The variable the supervisor writes and this crate reads.
#[test]
fn the_re_declared_variable_matches_the_platform() {
    assert_eq!(
        rivetos_addon::host::ADDONS_ENV,
        rivet_core::service::ADDONS_ENV
    );
}

/// **Both ends of the grammar, in one test.** The supervisor composes a value
/// and this crate parses it back; a change to either side that the other did
/// not follow fails here rather than on a board.
#[test]
fn what_the_supervisor_composes_is_what_this_crate_reads_back() {
    let manifest = rivet_core::package::application::Manifest::new(
        rivet_core::package::application::ProgramKind::Daemon,
    )
    .with_accepts_classes(vec!["aura-background-pack".to_string()])
    .expect("a one-entry accepted list");

    let library = |name: &str, byte: u8| rivet_core::service::InstalledLibrary {
        reference: rivet_core::package::library::Reference::parse(&format!(
            "{name}-1.0.0-{}",
            format!("{byte:02x}").repeat(32)
        ))
        .expect("a reference SPEC-02 accepts"),
        class: Some("aura-background-pack".to_string()),
        soname: Some(format!("lib{name}.so.1")),
    };
    let installed = [library("packone", 0x11), library("packtwo", 0x22)];

    let value = rivet_core::service::addon_env(
        rivet_core::service::BUILT_IN_VOLUME,
        &manifest,
        &installed,
    )
    .expect("two libraries of the accepted class joined");

    // The reader's half, driven through the same environment variable the
    // supervisor would have set. Serialised against nothing else: this test
    // binary runs it alone.
    //
    // SAFETY: an integration test is its own process and no other test in this
    // file touches the environment.
    unsafe { std::env::set_var(rivetos_addon::host::ADDONS_ENV, &value) };
    let found = rivetos_addon::host::installed("aura-background-pack");

    assert_eq!(found.len(), 2, "both packs come back: {value}");
    assert_eq!(found[0].soname(), "libpackone.so.1");
    assert_eq!(found[1].soname(), "libpacktwo.so.1");
    for entry in &found {
        assert_eq!(entry.class, "aura-background-pack");
        assert!(
            entry.path.is_absolute(),
            "a host opens this by path: {}",
            entry.path.display()
        );
        assert_eq!(
            entry.path,
            PathBuf::from(format!(
                "{}/{}",
                rivet_core::package::install::library_object_dir(
                    rivet_core::service::BUILT_IN_VOLUME,
                    &entry
                        .path
                        .parent()
                        .and_then(|p| p.parent())
                        .and_then(|p| p.file_name())
                        .and_then(std::ffi::OsStr::to_str)
                        .and_then(|name| rivet_core::package::library::Reference::parse(name).ok())
                        .expect("the path's directory is the library's reference")
                ),
                entry.soname()
            )),
            "the path is exactly the install layout SPEC-02 §4 describes"
        );
    }
}
