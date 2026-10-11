//! **Stamp the soname a host will ask for.**
//!
//! Cargo names the file `libaura_background_pack.so` and a host asks the
//! loader for `libaura-background.so.1`. The file is renamed when the
//! artifact is packaged, and `DT_SONAME` has to say the same thing or the
//! object registers under one name while the host asked for another — which
//! works, and then stops working the first time something else pulls the
//! same object in by its real soname.
//!
//! `examples/probes/frame-probe/build.rs` and
//! `examples/probes/addon-probe/build.rs` are the same four lines for the
//! same reason, and the three are deliberately not shared: a build script is
//! four lines, and a crate that reached across the tree for them would be a
//! dependency an addon author copying this pack could not satisfy.

fn main() {
    println!("cargo::rustc-link-arg=-Wl,-soname,libaura-background.so.1");
    println!("cargo::rerun-if-changed=build.rs");
}
