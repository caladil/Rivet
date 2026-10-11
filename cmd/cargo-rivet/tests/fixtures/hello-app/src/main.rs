//! The smallest thing that is a Rivet application: a static binary that says
//! something and exits.
//!
//! `tests/roundtrip.rs` packages this crate with `--binary`, so the suite does
//! not need a cross toolchain to run. Built for real with:
//!
//! ```text
//! cargo rivet package --manifest-path cmd/cargo-rivet/tests/fixtures/hello-app/Cargo.toml \
//!     --signing-key <a key you made> --out-dir <somewhere>
//! ```
//!
//! **Nothing on a Rivet can start it yet** — the supervisor's service list is
//! a compile-time constant and the desired-state document cannot name a fourth
//! service (`PLAN.md` §0.8, link 7). It prints rather than serving for that
//! reason: a daemon nothing can supervise is a process nothing would ever stop.

fn main() {
    println!("hello from a Rivet application");
}
