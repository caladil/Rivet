//! **The crossing itself: three symbols, four rules, and the macro that means
//! an addon author writes none of it.**
//!
//! # Why three symbols and not one
//!
//! [`VERSION_SYMBOL`] is read **before any bytes are exchanged**. A host that
//! cannot speak an addon's version refuses it having parsed nothing the addon
//! wrote — which is the difference between rejecting a stranger at the door
//! and rejecting one in the hallway. [`CALL_SYMBOL`] carries every message.
//! [`FREE_SYMBOL`] exists because only the side that allocated a buffer can
//! safely release it: two Rust builds can hold two allocators, and `free`ing
//! across them is heap corruption with a delay on it.
//!
//! # And two more, for the half that draws
//!
//! [`crate::frame`] adds [`crate::frame::FRAME_VERSION_SYMBOL`] and
//! [`crate::frame::DRAW_SYMBOL`], which carry a `#[repr(C)]` descriptor rather
//! than serialized bytes — `.rulings/` R-199 D2, and a deliberate exception to
//! the rule stated at the top of [`crate`]. The argument for it is arithmetic
//! and the argument for its narrowness is the C ABI; both are in that module's
//! header. An addon that measures rather than draws exports the three symbols
//! here and nothing else.
//!
//! # How an addon is found
//!
//! **The supervisor names it.** `ADR-053` puts every class-matched library's
//! directory on the host's `LD_LIBRARY_PATH`, and since 2026-09-19 it also
//! puts every one of those libraries' class and object into `RIVET_ADDONS` —
//! `rivet_core::service::ADDONS_ENV`, read back by
//! [`crate::host::installed`]. A host therefore learns **which** objects it
//! was joined to and opens one by path.
//!
//! Nothing about that scans a directory the supervisor did not name, which is
//! `ADR-011`'s line: the supervisor decides what joins, and the host is told.
//!
//! # The naming rule that used to do this, and why it went
//!
//! Until 2026-09-19 the supervisor named nothing, so the missing half was a
//! naming rule: an addon of class `c` was the object [`soname_for_class`]
//! returns, `libc.so.1`-style. **That rule makes two libraries of one class
//! one filename**, and `.rulings/` R-199 D1 makes a second background pack
//! the case to support — so it could not stay. It survives as
//! [`crate::host::Loaded::open`]'s fallback and nowhere else: an application
//! is installed independently of the image that supervises it, so a host
//! built today can run on a board whose supervisor names nothing, and on that
//! board the derived name is still what the loader will answer to.
//!
//! # The four rules the glue enforces
//!
//! 1. **The request is copied on entry.** Nothing is borrowed across the
//!    boundary, so the host is free to release its buffer the instant the call
//!    returns.
//! 2. **The answer is allocated by the addon and freed by the addon**, through
//!    [`FREE_SYMBOL`], with a length the host hands back unchanged.
//! 3. **Nothing unwinds.** Every entry point is wrapped in
//!    [`std::panic::catch_unwind`]; unwinding out of an `extern "C"` function
//!    is undefined behaviour, and a panicking addon must cost its answer
//!    rather than the host's process.
//! 4. **A failure still answers.** A panic or a bad request comes back as a
//!    serialized [`crate::Response::Fault`], not as a null pointer, so a host
//!    has something to put in the journal.

use crate::protocol::{Description, Failure, Polled, Request, Response};

/// The protocol version. Bumped when a change would make an older host
/// misread a newer addon — never for a new optional field, which
/// `a_field_this_build_does_not_know_is_ignored_rather_than_refused` proves is
/// already free.
pub const VERSION: u32 = 1;

/// `unsafe extern "C" fn() -> u32`
pub const VERSION_SYMBOL: &[u8] = b"rivet_addon_abi\0";

/// `unsafe extern "C" fn(*const u8, usize, *mut *mut u8, *mut usize) -> i32`
///
/// Returns `0` when `*out` holds `*out_len` bytes of answer, and non-zero when
/// the addon could not produce even a fault — in which case nothing was
/// allocated and nothing needs freeing.
pub const CALL_SYMBOL: &[u8] = b"rivet_addon_call\0";

/// `unsafe extern "C" fn(*mut u8, usize)`
pub const FREE_SYMBOL: &[u8] = b"rivet_addon_free\0";

/// The object a host asks the loader for **when the machine named nothing** —
/// [`crate::host::Loaded::open`]'s fallback, and this module's header for why
/// it is a fallback rather than the rule.
///
/// `ADR-053` §4 leaves a class as free text on the argument that the two
/// authors already coordinate on every other byte of the join. This function
/// is where that becomes a filename, so [`class_is_nameable`] is what a class
/// has to satisfy before it can be one.
///
/// **It is still what an addon's `build.rs` stamps into `DT_SONAME`**, which
/// is the other reason it is kept: an object whose soname is derived from its
/// class is one an older host can find, and there is no cost to a newer host
/// that opens it by path anyway.
#[must_use]
pub fn soname_for_class(class: &str) -> String {
    format!("lib{class}.so.1")
}

/// Whether a class can be half of a filename.
///
/// Lowercase letters, digits and a dash, not starting or ending with one —
/// the alphabet `SETTING_RULE` already uses for a short machine-read token,
/// reused rather than invented. A class outside it is refused when the addon
/// is described, not when the loader silently fails to find an object whose
/// name contains a slash.
#[must_use]
pub fn class_is_nameable(class: &str) -> bool {
    !class.is_empty()
        && !class.starts_with('-')
        && !class.ends_with('-')
        && class
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// **What an addon author implements.** One safe trait; the `extern "C"` half
/// is [`crate::export_addon!`]'s.
///
/// Every method takes `&mut self` because an addon holds what it has measured
/// between calls — the reading and the instant it was taken — and because the
/// host serializes calls behind a lock, so there is no concurrency here to
/// design for.
pub trait Addon: Send + 'static {
    /// Called once per load, after the version handshake.
    fn describe(&mut self) -> Description;

    /// Called before the first poll and again whenever the operator saves.
    ///
    /// # Errors
    /// One sentence for a person when the addon cannot work with what it was
    /// given. The host keeps it loaded and shows the complaint: an operator
    /// fixing a setting is the expected next event, not a reason to unload.
    fn configure(
        &mut self,
        settings: &serde_json::Map<String, serde_json::Value>,
        material: &std::collections::BTreeMap<String, String>,
    ) -> Result<(), String>;

    /// Called when this addon's own previous `call_again_in_ms` has elapsed.
    fn poll(&mut self, now_ms: i64) -> Polled;
}

/// Dispatch one decoded request against an addon.
///
/// Split out of the macro so that it is ordinary, testable Rust rather than
/// something that only exists after expansion — the macro's body is then four
/// lines of glue, which is the amount a reader can check.
pub fn dispatch<A: Addon>(addon: &mut A, request: &[u8]) -> Vec<u8> {
    let decoded = match crate::protocol::decode::<Request>(request) {
        Ok(envelope) if envelope.v == VERSION => envelope.body,
        Ok(envelope) => {
            return fault(
                "unsupported-version",
                &format!(
                    "this addon speaks version {VERSION} and the request said version {}",
                    envelope.v
                ),
            );
        }
        Err(e) => return fault("unreadable-request", &e.to_string()),
    };

    let answer = match decoded {
        Request::Describe => Response::Described(addon.describe()),
        Request::Configure { settings, material } => match addon.configure(&settings, &material) {
            Ok(()) => Response::Configured {
                usable: true,
                complaint: String::new(),
            },
            Err(complaint) => Response::Configured {
                usable: false,
                complaint,
            },
        },
        Request::Poll { now_ms } => Response::Polled(addon.poll(now_ms)),
    };

    crate::protocol::encode(answer).unwrap_or_else(|e| fault("unencodable-answer", &e.to_string()))
}

/// A fault, already encoded. Never fails: the fallback is a hand-written
/// object, because the one thing this function may not do is return nothing.
#[must_use]
pub fn fault(kind: &str, sentence: &str) -> Vec<u8> {
    crate::protocol::encode(Response::Fault(Failure {
        kind: kind.to_string(),
        sentence: sentence.to_string(),
    }))
    .unwrap_or_else(|_| {
        br#"{"v":1,"answer":"fault","kind":"unencodable","sentence":"the fault itself could not be encoded"}"#
            .to_vec()
    })
}

/// **Write an addon's three C symbols.**
///
/// ```ignore
/// rivetos_addon::export_addon!(ThermometerAddon);
/// ```
///
/// The type must implement [`Addon`] and [`Default`]; the instance is created
/// on first call and lives for the life of the load, behind a `Mutex` so that
/// a host calling from a worker thread — which it should, so that a hanging
/// addon blocks nothing that matters — cannot produce two `&mut` at once.
///
/// **A poisoned lock is an answer, not a panic.** If a previous call panicked
/// while holding it, every later call returns a fault naming that, which is
/// how an addon that broke once stops pretending it is fine.
///
/// # The `cdylib` this expands into MUST be built with `panic = "unwind"`
///
/// This macro writes [`std::panic::catch_unwind`] around every call, so a
/// panicking addon returns a fault instead of taking down whatever loaded
/// it. That guarantee holds only when the addon `cdylib` **itself** — not
/// its host, and not the workspace it happens to sit in — is compiled with
/// `panic = "unwind"`. With `panic = "abort"`, the addon's own copy of
/// `catch_unwind` has nothing to catch: the process aborts at the point of
/// the panic, before unwinding ever starts, and the host that loaded the
/// addon goes down with it.
///
/// `panic = "abort"` is a common choice for a plain binary — smaller, and
/// slightly faster to build — which is exactly what makes it a trap here:
/// copying a binary's `[profile.release]` into a new addon crate compiles
/// cleanly and passes every test that does not provoke a panic. Put this in
/// the addon crate's own `Cargo.toml` (`examples/probes/addon-probe`,
/// `examples/probes/frame-probe` and `examples/aura/aura-background-pack`
/// all carry it, with this same reasoning):
///
/// ```toml
/// [profile.release]
/// panic = "unwind"
/// ```
///
/// `panic` is a profile setting Cargo reads only from a WORKSPACE ROOT, so
/// it has to be set in the addon crate's own manifest AND that manifest has
/// to be its own workspace root — a member crate's `[profile.*]` is ignored
/// in favour of whatever workspace it was swept into. This repository's own
/// root `Cargo.toml` sets `panic = "abort"` for its own binaries, because
/// PID 1 must never unwind out of `main` (`ADR-017` §3), so an addon crate
/// built from inside this repository without its own `[workspace]` table
/// would silently inherit exactly the wrong setting. That is why every
/// in-tree addon above declares a bare `[workspace]` in its own
/// `Cargo.toml`: it is not a leftover, it is what makes that crate's own
/// `panic = "unwind"` the one that actually applies.
#[macro_export]
macro_rules! export_addon {
    ($addon:ty) => {
        /// **The object's one instance**, created on first call and living
        /// for the life of the load.
        ///
        /// A function rather than a `static` inside the entry point, because
        /// there is more than one entry point: [`export_drawing_addon!`] adds
        /// a drawing entry and it has to reach **this** instance. It did not,
        /// for as long as it took to run the first article against it — the
        /// pack that drew two frames reported having drawn none, because the
        /// two `static`s were two packs. One instance is what makes the
        /// catalogue a host read and the ids it then sends talk about the
        /// same list.
        #[doc(hidden)]
        pub fn __rivet_addon_instance() -> &'static ::std::sync::Mutex<$addon> {
            static INSTANCE: ::std::sync::OnceLock<::std::sync::Mutex<$addon>> =
                ::std::sync::OnceLock::new();
            INSTANCE.get_or_init(|| {
                ::std::sync::Mutex::new(<$addon as ::std::default::Default>::default())
            })
        }

        /// The protocol version this object was built against.
        #[unsafe(no_mangle)]
        pub extern "C" fn rivet_addon_abi() -> u32 {
            $crate::abi::VERSION
        }

        /// The one call. See `rivetos_addon::abi` for the four rules.
        ///
        /// # Safety
        /// `request` must point at `request_len` readable bytes, and `out` and
        /// `out_len` must be writable. The host's side of this contract is
        /// `rivetos_addon::host`.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn rivet_addon_call(
            request: *const u8,
            request_len: usize,
            out: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32 {
            if request.is_null() || out.is_null() || out_len.is_null() {
                return 2;
            }
            // RULE 1: copied on entry, so nothing is borrowed across the line.
            let owned: Vec<u8> =
                unsafe { ::std::slice::from_raw_parts(request, request_len) }.to_vec();

            // RULE 3: nothing unwinds out of an `extern "C"` function.
            let answer = ::std::panic::catch_unwind(move || {
                match __rivet_addon_instance().lock() {
                    Ok(mut guard) => $crate::abi::dispatch(&mut *guard, &owned),
                    Err(_) => $crate::abi::fault(
                        "poisoned",
                        "an earlier call panicked while holding this addon, so it is not \
                         answering again until it is reloaded",
                    ),
                }
            })
            // RULE 4: a panic is still an answer.
            .unwrap_or_else(|_| {
                $crate::abi::fault("panicked", "this addon panicked while answering")
            });

            // RULE 2: allocated here, released by `rivet_addon_free`.
            // `into_boxed_slice` makes capacity equal length, which is what
            // lets the free side rebuild the allocation from a pointer and a
            // length alone.
            let boxed: ::std::boxed::Box<[u8]> = answer.into_boxed_slice();
            let len = boxed.len();
            let ptr = ::std::boxed::Box::into_raw(boxed).cast::<u8>();
            unsafe {
                *out = ptr;
                *out_len = len;
            }
            0
        }

        /// Release what `rivet_addon_call` returned.
        ///
        /// # Safety
        /// `ptr` and `len` must be exactly what one call returned, handed back
        /// once.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn rivet_addon_free(ptr: *mut u8, len: usize) {
            if ptr.is_null() {
                return;
            }
            let _ = ::std::panic::catch_unwind(|| unsafe {
                drop(::std::boxed::Box::from_raw(
                    ::std::ptr::slice_from_raw_parts_mut(ptr, len),
                ));
            });
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Envelope, Status};

    #[derive(Default)]
    struct Stub {
        configured: bool,
    }

    impl Addon for Stub {
        fn describe(&mut self) -> Description {
            Description {
                name: "stub".to_string(),
                version: "0.0.0".to_string(),
                class: "test-source".to_string(),
                fields: Vec::new(),
                material: Vec::new(),
                backgrounds: Vec::new(),
            }
        }
        fn configure(
            &mut self,
            settings: &serde_json::Map<String, serde_json::Value>,
            _material: &std::collections::BTreeMap<String, String>,
        ) -> Result<(), String> {
            if settings.contains_key("break") {
                return Err("told to break".to_string());
            }
            self.configured = true;
            Ok(())
        }
        fn poll(&mut self, _now_ms: i64) -> Polled {
            Polled {
                reading: None,
                status: if self.configured {
                    Status::Ok
                } else {
                    Status::Unavailable
                },
                call_again_in_ms: 1_000,
                complaint: String::new(),
                also: Vec::new(),
            }
        }
    }

    fn answer(bytes: &[u8]) -> Response {
        crate::protocol::decode::<Response>(bytes)
            .expect("an answer is always decodable")
            .body
    }

    #[test]
    fn a_request_from_a_newer_protocol_is_refused_by_name() {
        let mut stub = Stub::default();
        let request = br#"{"v":9999,"op":"describe"}"#;
        match answer(&dispatch(&mut stub, request)) {
            Response::Fault(f) => {
                assert_eq!(f.kind, "unsupported-version");
                assert!(f.sentence.contains("9999"), "names it: {}", f.sentence);
            }
            other => panic!("read as {other:?}"),
        }
    }

    #[test]
    fn bytes_that_are_not_a_request_come_back_as_a_fault_rather_than_nothing() {
        let mut stub = Stub::default();
        match answer(&dispatch(&mut stub, b"not json at all")) {
            Response::Fault(f) => assert_eq!(f.kind, "unreadable-request"),
            other => panic!("read as {other:?}"),
        }
    }

    #[test]
    fn a_refused_configuration_keeps_the_addon_loaded_and_says_why() {
        let mut stub = Stub::default();
        let request = crate::protocol::encode(Request::Configure {
            settings: serde_json::from_str(r#"{"break":true}"#).expect("literal"),
            material: std::collections::BTreeMap::new(),
        })
        .expect("encodes");
        match answer(&dispatch(&mut stub, &request)) {
            Response::Configured { usable, complaint } => {
                assert!(!usable);
                assert_eq!(complaint, "told to break");
            }
            other => panic!("read as {other:?}"),
        }
        // Still answering: a refused configuration is not an unload.
        let poll = crate::protocol::encode(Request::Poll { now_ms: 0 }).expect("encodes");
        assert!(matches!(
            answer(&dispatch(&mut stub, &poll)),
            Response::Polled(_)
        ));
    }

    #[test]
    fn a_class_becomes_the_object_the_loader_is_asked_for() {
        assert_eq!(soname_for_class("aura-reading-source"), "libaura-reading-source.so.1");
        assert!(class_is_nameable("aura-reading-source"));
        assert!(!class_is_nameable("../etc/passwd"));
        assert!(!class_is_nameable("Aura"), "an upper-case class is refused");
        assert!(!class_is_nameable("-leading"));
        assert!(!class_is_nameable(""));
    }

    #[test]
    fn the_envelope_carries_the_version_this_build_speaks() {
        let bytes = crate::protocol::encode(Request::Describe).expect("encodes");
        let back: Envelope<Request> = crate::protocol::decode(&bytes).expect("decodes");
        assert_eq!(back.v, VERSION);
    }

    /// An addon whose every method panics, on purpose, so the test below can
    /// call the real generated glue rather than [`dispatch`] and watch it
    /// survive.
    ///
    /// `pub(crate)`, not private: [`export_addon!`] expands to a `pub fn`
    /// returning `&'static Mutex<Grenade>`, and a private type there is a
    /// private-in-public-interface — the macro cannot itself downgrade the
    /// visibility it writes for the general case just because this one
    /// invocation is test-only.
    #[derive(Default)]
    pub(crate) struct Grenade;

    impl Addon for Grenade {
        fn describe(&mut self) -> Description {
            panic!("Grenade::describe always panics, on purpose, for this test")
        }
        fn configure(
            &mut self,
            _settings: &serde_json::Map<String, serde_json::Value>,
            _material: &std::collections::BTreeMap<String, String>,
        ) -> Result<(), String> {
            Ok(())
        }
        fn poll(&mut self, _now_ms: i64) -> Polled {
            panic!("Grenade::poll always panics, on purpose, for this test")
        }
    }

    // The real macro expansion — `rivet_addon_call` and friends, exactly as
    // an addon author's own crate gets them from
    // `rivetos_addon::export_addon!` — not a hand-simplified stand-in.
    crate::export_addon!(Grenade);

    /// **The generated glue's own promise, exercised for real.** This calls
    /// the actual `unsafe extern "C" fn rivet_addon_call` [`export_addon!`]
    /// wrote above — not [`dispatch`] directly — the same way a host would.
    ///
    /// It proves the `catch_unwind` half of the contract: an [`Addon`] whose
    /// method panics answers `Response::Fault { kind: "panicked", .. }`
    /// rather than unwinding out of `rivet_addon_call`.
    ///
    /// **Removing the `catch_unwind` does not merely fail this test — it
    /// aborts the whole test process**, confirmed by deleting it while
    /// proving this test red: the panic tries to cross `rivet_addon_call`'s
    /// plain `extern "C"` boundary, which is not unwind-safe, and Rust's own
    /// runtime reports *"panic in a function that cannot unwind"* and calls
    /// `abort` before this crate's `catch_unwind` would even matter. That is
    /// the exact failure mode this whole macro exists to prevent, observed
    /// directly rather than only reasoned about — this in-process test does
    /// not need a separately built `panic = "abort"` `cdylib` to demonstrate
    /// it, because an uncaught panic is already fatal at this boundary on
    /// its own. What it does NOT cover is the *other* way to reach the same
    /// abort — a `cdylib` built with `panic = "abort"` in the first place,
    /// where `catch_unwind` has nothing to catch because no unwinding
    /// machinery was even linked in. That variant can only be observed by
    /// building a real `cdylib` with `panic = "abort"`, loading it, and
    /// watching the host die with it; `export_addon!`'s doc section and
    /// every in-tree addon's `Cargo.toml` exist to prevent it by
    /// construction rather than by test.
    #[test]
    fn a_panicking_addon_call_answers_a_fault_instead_of_unwinding_out() {
        let request = crate::protocol::encode(Request::Poll { now_ms: 0 }).expect("encodes");
        let mut out: *mut u8 = std::ptr::null_mut();
        let mut out_len: usize = 0;
        // Safety: `request` is a live buffer of `request.len()` bytes; `out`
        // and `out_len` are valid local places to write through — exactly
        // `rivet_addon_call`'s documented contract.
        let rc = unsafe {
            rivet_addon_call(request.as_ptr(), request.len(), &raw mut out, &raw mut out_len)
        };
        assert_eq!(rc, 0, "a panic must still produce an answer, not fail the call itself");
        assert!(!out.is_null());
        // Safety: `out`/`out_len` are exactly what the call above just wrote.
        let bytes = unsafe { std::slice::from_raw_parts(out, out_len) }.to_vec();
        match answer(&bytes) {
            Response::Fault(f) => assert_eq!(f.kind, "panicked"),
            other => panic!("a panicking addon should answer Fault(panicked), got {other:?}"),
        }
        // Safety: exactly what `rivet_addon_call` returned, handed back once,
        // as `rivet_addon_free`'s contract requires.
        unsafe { rivet_addon_free(out, out_len) };
    }
}
