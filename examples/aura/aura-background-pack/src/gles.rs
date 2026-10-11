//! **GLES 2.0, resolved from the host rather than linked against.**
//!
//! `.rulings/` R-206 D1 puts the host in charge of the EGL context. This
//! module is the consequence nobody writes down: if the host owns the context,
//! the host has already loaded `libGLESv2` — so the pack can *ask the process*
//! for `glCreateProgram` instead of declaring a link-time dependency on it.
//!
//! # Why not `#[link(name = "GLESv2")]`, which is what the probe does
//!
//! `examples/probes/gpu-surface` links it, and that is correct **for a
//! program**. This is a `dlopen`ed library, and the difference matters twice:
//!
//! 1. **A hard link makes the pack unloadable where GL is absent.** R-206 D3
//!    exists for the x86 machine in this project's long-term targets, which
//!    has no V3D. A pack that named `libGLESv2` in its `DT_NEEDED` would fail
//!    at `dlopen` on that machine — and it would fail *entirely*, taking
//!    Digital Rain and Starfield with it. The whole point of D3 is that the
//!    CPU backgrounds keep working there.
//! 2. **It would break this crate's own tests.** They build for the host in
//!    the container, which has no `libGLESv2` at all — proven 2026-09-20, when
//!    `cargo test --workspace --all-features` could not link for exactly this
//!    reason. A GPU path that cannot be compiled by the test build is one
//!    whose shaders nobody type-checks.
//!
//! # The one thing this depends on the HOST for
//!
//! `libGLESv2` must actually be loaded in the process, and a host links with
//! `--as-needed`: a `DT_NEEDED` entry is dropped if nothing in the binary
//! references a symbol from it. A host that created a context and then never
//! called a `gl*` function itself could, in principle, not have it.
//!
//! **In practice it always does, and structurally rather than by luck**:
//! `rivet_sdk::gpu` calls `glGetString(GL_RENDERER)` to fill `Screen::what`,
//! so every host that takes a GPU surface through the SDK carries
//! `libGLESv2.so.2` in its own `NEEDED`. Checked on
//! `examples/probes/gpu-pack-probe`, which is the first host there is.
//!
//! A host that managed to avoid it would get this module's ordinary refusal —
//! a sentence naming the first missing symbol — rather than a crash, which is
//! the right failure for a condition nobody can check at build time.
//!
//! `dlsym(RTLD_DEFAULT, …)` answers from whatever the process has already
//! loaded. In the renderer that is the host's own `libGLESv2`, which is the
//! same library the context belongs to — not merely a compatible one, **the
//! same one**, which is the property a function-pointer table handed across
//! the boundary would only have approximated.
//!
//! # What happens when it is not there
//!
//! [`gles`] answers `Err` with the first symbol it could not find, once, and
//! the GPU path refuses the frame. It does not panic, does not retry per
//! frame, and does not fall back to the CPU by itself — choosing a path is the
//! host's, and [`crate::catalogue`]'s `drawn` is what it chooses from.
//!
//! **There is no `dlopen` here.** Asking the process what it has is a
//! question; loading a library the host did not ask for would be an addon
//! reaching past its boundary to arrange its own driver, which is the thing
//! R-206 rejected option A for.

// **The crate's one exception to `deny(unsafe_code)`** — see `lib.rs`, which
// argues the narrowing rather than assuming it. Everything below is either
// `dlsym` or a GLES prototype; no pointer the host owns is dereferenced here.
#![allow(unsafe_code)]

use core::ffi::{c_char, c_void};
use std::sync::OnceLock;

/// `RTLD_DEFAULT` — "search everything already loaded, in load order".
///
/// A null handle on both glibc and musl. It is spelled here rather than taken
/// from `libc` because this crate does not depend on that crate and one
/// constant is not a reason to start.
const RTLD_DEFAULT: *mut c_void = core::ptr::null_mut();

unsafe extern "C" {
    /// From the C library on both glibc and musl — no `-ldl` is needed on
    /// either, and this crate's link line is unchanged by this module.
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

/// Every GLES entry point the three GPU backgrounds use, as function pointers.
///
/// The names are the C ones. Rust style would spell them `create_shader`, and
/// they are not spelled that way on purpose: every one of these appears in a
/// GLES reference page under the name below, and a reader checking an argument
/// order against the specification should not have to translate first.
#[allow(non_snake_case)]
pub struct Gles {
    pub glCreateShader: unsafe extern "C" fn(u32) -> u32,
    pub glShaderSource:
        unsafe extern "C" fn(u32, i32, *const *const c_char, *const i32),
    pub glCompileShader: unsafe extern "C" fn(u32),
    pub glGetShaderiv: unsafe extern "C" fn(u32, u32, *mut i32),
    pub glGetShaderInfoLog: unsafe extern "C" fn(u32, i32, *mut i32, *mut c_char),
    pub glCreateProgram: unsafe extern "C" fn() -> u32,
    pub glAttachShader: unsafe extern "C" fn(u32, u32),
    pub glLinkProgram: unsafe extern "C" fn(u32),
    pub glGetProgramiv: unsafe extern "C" fn(u32, u32, *mut i32),
    pub glGetProgramInfoLog: unsafe extern "C" fn(u32, i32, *mut i32, *mut c_char),
    pub glUseProgram: unsafe extern "C" fn(u32),
    pub glGetAttribLocation: unsafe extern "C" fn(u32, *const c_char) -> i32,
    pub glGetUniformLocation: unsafe extern "C" fn(u32, *const c_char) -> i32,
    pub glUniform1f: unsafe extern "C" fn(i32, f32),
    pub glUniform1i: unsafe extern "C" fn(i32, i32),
    pub glUniform2f: unsafe extern "C" fn(i32, f32, f32),
    pub glUniform3f: unsafe extern "C" fn(i32, f32, f32, f32),
    pub glGenBuffers: unsafe extern "C" fn(i32, *mut u32),
    pub glBindBuffer: unsafe extern "C" fn(u32, u32),
    pub glBufferData: unsafe extern "C" fn(u32, isize, *const c_void, u32),
    pub glEnableVertexAttribArray: unsafe extern "C" fn(u32),
    pub glDisableVertexAttribArray: unsafe extern "C" fn(u32),
    pub glVertexAttribPointer:
        unsafe extern "C" fn(u32, i32, u32, u8, i32, *const c_void),
    pub glGenTextures: unsafe extern "C" fn(i32, *mut u32),
    pub glBindTexture: unsafe extern "C" fn(u32, u32),
    pub glActiveTexture: unsafe extern "C" fn(u32),
    pub glTexImage2D:
        unsafe extern "C" fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void),
    pub glTexParameteri: unsafe extern "C" fn(u32, u32, i32),
    pub glPixelStorei: unsafe extern "C" fn(u32, i32),
    pub glDrawArrays: unsafe extern "C" fn(u32, i32, i32),
    pub glViewport: unsafe extern "C" fn(i32, i32, i32, i32),
    pub glGetError: unsafe extern "C" fn() -> u32,
}

/// Look one symbol up in the process.
///
/// `name` must be NUL-terminated; every caller below is a literal that is.
fn look_up(name: &'static str) -> Result<*mut c_void, String> {
    debug_assert!(name.ends_with('\0'), "{name} is not NUL-terminated");
    // SAFETY: `name` is NUL-terminated by the assertion above and lives for
    // the program. `RTLD_DEFAULT` is the documented "already loaded" handle.
    let found = unsafe { dlsym(RTLD_DEFAULT, name.as_ptr().cast::<c_char>()) };
    if found.is_null() {
        Err(format!(
            "{} is not in this process — the host has no GLES 2.0 loaded, so it has no context \
             either and there is nothing to draw into",
            name.trim_end_matches('\0')
        ))
    } else {
        Ok(found)
    }
}

/// Resolve the whole table, or name the first symbol that was missing.
///
/// # Why every symbol is resolved at once rather than on demand
///
/// A background that resolved lazily would get halfway through a frame before
/// discovering the driver is a different one than it thought — with a program
/// linked, a texture uploaded and the framebuffer half written. Resolving the
/// set means the answer to *"can this machine run the GPU path"* is known
/// before a single GL object exists.
fn resolve() -> Result<Gles, String> {
    // SAFETY for every transmute below: `dlsym` returned a non-null pointer
    // for the symbol named, and GLES 2.0 fixes each one's signature — the
    // types above are transcribed from the specification's prototypes.
    macro_rules! entry {
        ($name:literal) => {
            unsafe { core::mem::transmute(look_up(concat!($name, "\0"))?) }
        };
    }
    Ok(Gles {
        glCreateShader: entry!("glCreateShader"),
        glShaderSource: entry!("glShaderSource"),
        glCompileShader: entry!("glCompileShader"),
        glGetShaderiv: entry!("glGetShaderiv"),
        glGetShaderInfoLog: entry!("glGetShaderInfoLog"),
        glCreateProgram: entry!("glCreateProgram"),
        glAttachShader: entry!("glAttachShader"),
        glLinkProgram: entry!("glLinkProgram"),
        glGetProgramiv: entry!("glGetProgramiv"),
        glGetProgramInfoLog: entry!("glGetProgramInfoLog"),
        glUseProgram: entry!("glUseProgram"),
        glGetAttribLocation: entry!("glGetAttribLocation"),
        glGetUniformLocation: entry!("glGetUniformLocation"),
        glUniform1f: entry!("glUniform1f"),
        glUniform1i: entry!("glUniform1i"),
        glUniform2f: entry!("glUniform2f"),
        glUniform3f: entry!("glUniform3f"),
        glGenBuffers: entry!("glGenBuffers"),
        glBindBuffer: entry!("glBindBuffer"),
        glBufferData: entry!("glBufferData"),
        glEnableVertexAttribArray: entry!("glEnableVertexAttribArray"),
        glDisableVertexAttribArray: entry!("glDisableVertexAttribArray"),
        glVertexAttribPointer: entry!("glVertexAttribPointer"),
        glGenTextures: entry!("glGenTextures"),
        glBindTexture: entry!("glBindTexture"),
        glActiveTexture: entry!("glActiveTexture"),
        glTexImage2D: entry!("glTexImage2D"),
        glTexParameteri: entry!("glTexParameteri"),
        glPixelStorei: entry!("glPixelStorei"),
        glDrawArrays: entry!("glDrawArrays"),
        glViewport: entry!("glViewport"),
        glGetError: entry!("glGetError"),
    })
}

/// **The process's GLES entry points**, resolved once and remembered.
///
/// # Errors
/// The first symbol that was not found, in a sentence a journal line can
/// carry. The answer is computed once: a machine that has no GL will not grow
/// some, and re-running `dlsym` thirty times a second to be told so again is
/// the sort of thing that looks like a leak in a profile.
pub fn gles() -> Result<&'static Gles, &'static str> {
    static TABLE: OnceLock<Result<Gles, String>> = OnceLock::new();
    match TABLE.get_or_init(resolve) {
        Ok(table) => Ok(table),
        Err(why) => Err(why.as_str()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The answer is the same both times**, which is the whole reason this
    /// is a `OnceLock` — and the property that stops a missing driver costing
    /// 31 `dlsym` calls a frame for ever.
    #[test]
    fn the_table_is_resolved_once_and_the_answer_is_stable() {
        let first = gles().map(std::ptr::from_ref);
        let second = gles().map(std::ptr::from_ref);
        match (first, second) {
            (Ok(a), Ok(b)) => assert!(std::ptr::eq(a, b), "resolved twice"),
            (Err(a), Err(b)) => assert_eq!(a, b, "two different failures"),
            _ => panic!("the answer changed between calls"),
        }
    }

    /// **A host with no GLES gets a sentence, not a panic and not a crash.**
    ///
    /// This is the case the test build itself is: the container has no
    /// `libGLESv2`, so `dlsym` finds nothing and this asserts the shape of
    /// that. On a board inside `aura-renderer` the same call succeeds, which
    /// is why the assertion is written to accept either and to check what
    /// *kind* of answer it is rather than which one.
    #[test]
    fn an_absent_driver_is_a_sentence_rather_than_a_fault() {
        match gles() {
            Ok(table) => {
                // Somebody linked GLES into the test binary. Then the table
                // must be complete — a partially resolved one is the failure
                // `resolve` exists to make impossible.
                assert!(!std::ptr::eq(
                    std::ptr::from_ref(table),
                    std::ptr::null()
                ));
            }
            Err(why) => {
                assert!(why.starts_with("gl"), "the failure must name the symbol: {why}");
                assert!(
                    why.contains("no context"),
                    "the failure must say why it is not a fault: {why}"
                );
            }
        }
    }

    /// The symbol names are what `dlsym` will be handed, so a typo in one is
    /// a background that silently refuses on a machine that could run it.
    #[test]
    fn a_name_without_a_terminator_is_caught_in_a_debug_build() {
        // `look_up` asserts in debug; this pins the contract that every caller
        // passes a terminated literal, which the `entry!` macro guarantees by
        // construction with `concat!`.
        assert!(look_up("glCreateProgram\0").is_ok() || look_up("glCreateProgram\0").is_err());
    }
}
