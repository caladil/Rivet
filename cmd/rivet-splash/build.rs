//! **Rebuild when the commit changes**, and nothing else.
//!
//! `main.rs` reads `RIVET_DESCRIBE` with `option_env!`, which is baked in at
//! compile time. Cargo does not track environment variables for rebuild unless
//! it is told to, so without this a second card build would embed the FIRST
//! build's commit and put a stale version on the screen — which is worse than
//! showing none, because a version on a television is read as a fact.
fn main() {
    println!("cargo::rerun-if-env-changed=RIVET_DESCRIBE");
}
