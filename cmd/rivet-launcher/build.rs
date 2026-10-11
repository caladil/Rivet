//! Link `libdrm` **statically**, so the launcher stays a static binary.
//!
//! `PLAN.md` §104.36. `ADR-003` §5 makes every Rivet binary static, and this one
//! needs a modeset, which means `libdrm`. Those look like they contradict and do
//! not: Buildroot is configured `BR2_SHARED_STATIC_LIBS=y` since §104.17, so it
//! stages `libdrm.a` beside `libdrm.so.2` and this links the archive.
//!
//! # Why a `build.rs` and not a `.cargo/config.toml`
//!
//! `examples/probes/mesa-demo` and `examples/probes/kiosk-demo` each carry the link recipe in
//! their own `.cargo/config.toml`, and that works because each is its own
//! workspace root. **This crate is a member of the main workspace**, and cargo
//! reads that file from the invocation directory upward — so a per-crate one
//! here is simply ignored, and putting the flags in the workspace's would make
//! **every** Rivet binary dynamic. That is the rule `ADR-003` §5 exists to hold.
//!
//! A `build.rs` emits link directives for **one crate**, which is exactly the
//! scope wanted.
//!
//! # Why the path is the container's
//!
//! `ADR-026` §7 makes the build container the Linux side of this project and
//! `tools/container/exec.sh` mounts the Buildroot tree at `/br`. A host build of
//! this crate for the host's own target does not link `libdrm` at all — see the
//! target check below — so the path is only used where it exists.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    // **Only for the board.** `cargo check` on a developer's machine builds this
    // for the host, where there is no Buildroot tree and no need for one: the
    // FFI declarations type-check without a library to resolve them against.
    // Emitting the link flags unconditionally would make a host check fail on a
    // missing file, which is a worse experience than a check that simply does
    // not link.
    let target = std::env::var("TARGET").unwrap_or_default();
    if !target.contains("aarch64") || !target.contains("musl") {
        return;
    }

    // Where Buildroot stages the archive. The RUNTIME copy of the shared object
    // is irrelevant here: nothing about this binary is dynamic.
    println!("cargo:rustc-link-search=native=/br/staging/usr/lib");
    println!("cargo:rustc-link-lib=static=drm");
}
