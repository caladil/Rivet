//! **Stamp the soname a host will ask for.**
//!
//! Cargo names the file `libprobe_source.so`; `rivetos_addon::abi::
//! soname_for_class("probe-source")` says the loader will be asked for
//! `libprobe-source.so.1`. The file is renamed when the artifact is packaged,
//! and `DT_SONAME` has to say the same thing or the loader registers the
//! object under one name while the host asked for another — which works, and
//! then stops working the first time something else pulls the same object in
//! by its real soname.
//!
//! One line, and it is the reason an addon needs a `build.rs` at all.

fn main() {
    println!("cargo::rustc-link-arg=-Wl,-soname,libprobe-source.so.1");
    println!("cargo::rerun-if-changed=build.rs");
}
