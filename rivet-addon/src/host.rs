//! **The host's side: open an addon, agree a version, call it.**
//!
//! This file and [`crate::abi`]'s macro are the only `unsafe` in the addon
//! story, which is the point — an addon crate carries
//! `#![forbid(unsafe_code)]` and a host crate needs none of its own.
//!
//! # It never closes what it opened
//!
//! There is no `dlclose` here and that is deliberate. An object that has
//! registered a thread-local destructor, spawned a thread or installed an
//! `atexit` handler makes unloading unsound in ways that surface as a crash
//! long after the unload — and the host has no way to know whether it did.
//! An addon is loaded for the life of the process; changing one means
//! restarting the host, which is what `apply service-restart` is for.
//!
//! # A hang is the caller's problem to contain
//!
//! [`Loaded::call`] blocks until the addon returns. A host calls it from a
//! worker rather than from the loop that keeps its own service alive, so that
//! an addon which never returns costs a reading rather than the process —
//! `.rulings/` R-198 D3. Nothing here can enforce that, so it is said twice:
//! here, and on the trait the addon implements.
//!
//! # The frame is the same file's business, on different terms
//!
//! [`Loaded::draw`] hands an addon a `#[repr(C)]` descriptor over the host's
//! own surface instead of a serialized message — `.rulings/` R-199 D2, argued
//! in [`crate::frame`]. Everything above still holds for it, and one thing is
//! added: **nothing is allocated on either side of a frame**, so there is no
//! buffer to free and no sentence to read. What comes back is a code, and
//! [`DrawFailure`] is where this end turns it into prose.
//!
//! # How a host learns which objects it was joined to
//!
//! The supervisor names them, in [`ADDONS_ENV`], and [`installed`] reads it
//! back. Before 2026-09-19 it named none of them: `ADR-053` put every
//! class-matched library's *directory* on `LD_LIBRARY_PATH` and a host derived
//! the *filename* from the class, `lib<class>.so.1`. That rule cannot express
//! two libraries of one class — they are one filename — which is fine for a
//! reading source and wrong for a background pack, where a second pack is the
//! case to support (`.rulings/` R-199 D1, R-200 D2).

use std::ffi::CString;
use std::path::{Path, PathBuf};

use crate::abi;
use crate::frame;
use crate::gl;
use crate::protocol::{Envelope, Request, Response};

type CallFn = unsafe extern "C" fn(*const u8, usize, *mut *mut u8, *mut usize) -> i32;
type FreeFn = unsafe extern "C" fn(*mut u8, usize);
type VersionFn = unsafe extern "C" fn() -> u32;
type DrawFn = unsafe extern "C" fn(*const frame::Frame) -> i32;
type DrawGlFn = unsafe extern "C" fn(*const gl::GlFrame) -> i32;

/// **The variable the supervisor names its joins in** —
/// `rivet_core::service::ADDONS_ENV`, re-declared here because this crate
/// links no Rivet crate (`ADR-036` §5: an application is a foreign program).
/// `tests/agreement.rs` holds the two spellings together, exactly as
/// `rivet-sdk`'s does for every other handed value.
///
/// `<class>=<absolute path to the object>`, colon-separated, sorted. Absent
/// when no library of any accepted class is installed — and absent, too, on a
/// machine whose image predates the channel, which is why [`Loaded::open`]
/// still has a fallback.
pub const ADDONS_ENV: &str = "RIVET_ADDONS";

/// **One library the supervisor joined to this process**, as [`installed`]
/// reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// The class the library's own signed manifest declared — the same bytes
    /// the host's manifest accepted, because `SPEC-02` §11 makes matching
    /// exact.
    pub class: String,
    /// The object itself, absolute. [`Loaded::open_path`] takes this.
    pub path: PathBuf,
}

impl Installed {
    /// The object's file name, which is also its `DT_SONAME` — `cargo rivet
    /// library` refuses an object whose file name and soname disagree, so
    /// these are one string and not two.
    #[must_use]
    pub fn soname(&self) -> &str {
        self.path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or_default()
    }

    /// **What [`crate::trial`] compares a stale note against.**
    ///
    /// `SPEC-02` §3.1's library layout puts every reference at its own
    /// directory — `/state/lib/<name>-<version>-<digest>/lib/lib<class>.so.1`
    /// — so the grandparent of the object is `<name>-<version>-<digest>`, and
    /// it changes on every reinstall or upgrade because the digest does. That
    /// is exactly the property a trial note needs: a note written against one
    /// reference and read back against a different one names an object that
    /// changed since the crash, not the one that caused it.
    ///
    /// **Falls back to the object's own absolute path** when it is not laid
    /// out that way — a path a test or a probe composed by hand. A coarser
    /// identity is still an identity; the alternative would be treating every
    /// such case as though nothing were known at all, which is
    /// [`crate::trial::LeftBehind::Unidentified`]'s job and not this one's.
    #[must_use]
    pub fn identity(&self) -> String {
        self.path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(std::ffi::OsStr::to_str)
            .map_or_else(|| self.path.display().to_string(), str::to_owned)
    }
}

/// **Every library of this class the supervisor joined to this process**, in
/// the order it named them.
///
/// Empty is the ordinary answer on a machine with no addon of this class, and
/// also on one whose image predates the enumeration channel — those two are
/// told apart by [`Loaded::open`] and not here, because a *list* of what is
/// installed has the same content either way: nothing this function can name.
///
/// # It reads the environment and nothing else
///
/// No directory is scanned and no path is composed. `ADR-011`'s line: the
/// supervisor decides what joins, and a host that went looking would be a
/// second authority on the same question — which is the failure the naming
/// rule this replaces already had, one filename at a time.
#[must_use]
pub fn installed(class: &str) -> Vec<Installed> {
    let Ok(value) = std::env::var(ADDONS_ENV) else {
        return Vec::new();
    };
    joined(&value)
        .into_iter()
        .filter(|entry| entry.class == class)
        .collect()
}

/// Parse [`ADDONS_ENV`]'s value. Split out of [`installed`] so the grammar is
/// testable without an environment — the same split every other reader in this
/// project makes between what a value **means** and where it came **from**.
///
/// **An entry that is not `<class>=<path>` is skipped rather than refused.**
/// A host that stopped because one entry was malformed would be a host an
/// unrelated library could disable — and the entries are composed by the
/// supervisor from a matched list, so a malformed one says something has gone
/// wrong upstream rather than that this host should stop.
fn joined(value: &str) -> Vec<Installed> {
    value
        .split(':')
        .filter_map(|entry| {
            let (class, path) = entry.split_once('=')?;
            if class.is_empty() || path.is_empty() {
                return None;
            }
            Some(Installed {
                class: class.to_string(),
                path: PathBuf::from(path),
            })
        })
        .collect()
}

/// An addon that is open and agreed.
pub struct Loaded {
    class: String,
    object: PathBuf,
    call: CallFn,
    free: FreeFn,
    /// **`None` for an addon that does not draw**, which is every reading
    /// source. The presence of [`frame::DRAW_SYMBOL`] is the whole of what
    /// makes an object a drawing addon — see [`Loaded::draws`].
    draw: Option<DrawFn>,
    /// The descriptor version the object answered, when it draws. **Recorded
    /// here and compared at [`Loaded::draw`] rather than refused at the
    /// door**, deliberately: `aura-controller` opens a background pack to read
    /// its catalogue and never draws a pixel, so a pack whose frame layout
    /// this build cannot speak is still perfectly describable. Refusing the
    /// whole load over the drawing half would take a picker away from a host
    /// that was never going to draw.
    frame_abi: Option<u32>,
    /// **`None` for a pack with no GPU path**, which is every pack built
    /// before `.rulings/` R-206 and every pack that chose not to have one.
    /// [`gl::DRAW_GL_SYMBOL`]'s presence is the whole of it — see
    /// [`Loaded::draws_gl`].
    draw_gl: Option<DrawGlFn>,
    /// The GL descriptor version the object answered, when it has a GPU path.
    /// Recorded rather than refused at the door, for [`Loaded::frame_abi`]'s
    /// reason exactly: a controller reading a catalogue does not care.
    gl_abi: Option<u32>,
}

/// Why an addon was not opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenFailure {
    /// The class cannot be half of a filename — [`abi::class_is_nameable`].
    /// Only reachable on [`Loaded::open`]'s fallback, where a class still
    /// becomes a filename; a class the supervisor named is opened by path and
    /// never spelled into one.
    ClassNotNameable { class: String },
    /// **Nothing of this class was opened.** `looked_for` is the object the
    /// loader was asked for, or the class itself when the supervisor named no
    /// library of it; `reason` is `dlerror`'s own sentence in the first case,
    /// which names the missing dependency when that is what went wrong.
    ///
    /// **The ordinary answer on a machine with no addon of this class**, and
    /// not a fault.
    NotLoaded { looked_for: String, reason: String },
    /// The object loaded but is not an addon: one of the three symbols is
    /// missing.
    NotAnAddon { object: String, symbol: String },
    /// The object is an addon this host cannot speak to. **Read before any
    /// bytes were exchanged.**
    VersionMismatch {
        object: String,
        theirs: u32,
        ours: u32,
    },
    /// **Several libraries of this class are installed, and this is not the
    /// place to choose between them** — `.rulings/` R-200.
    ///
    /// A class names a role and there is one class per role, so two libraries
    /// of one class reaching a host that wants *the* addon of that class is an
    /// operator having installed two things for one job. Every one of them is
    /// named, because the operator's next question is *which two*, and a host
    /// that picked silently would answer it by accident on the next boot.
    ///
    /// A host that genuinely wants all of them — a renderer drawing through
    /// every background pack — calls [`installed`] and [`Loaded::open_path`]
    /// instead, which is what this variant exists to point at.
    Ambiguous { class: String, objects: Vec<String> },
}

impl std::fmt::Display for OpenFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClassNotNameable { class } => write!(
                f,
                "REFUSED the class {class:?} is not a name an object can have — lower-case \
                 letters, digits and a dash, not starting or ending with one"
            ),
            Self::NotLoaded { looked_for, reason } => write!(
                f,
                "no addon was loaded for {looked_for}: {reason}. This is the ordinary case on a \
                 machine with no addon installed, because ADR-053 joins a library by class and \
                 an absent one is silent"
            ),
            Self::NotAnAddon { object, symbol } => write!(
                f,
                "REFUSED {object} loaded but does not export {symbol}, so it is a library rather \
                 than an addon"
            ),
            Self::VersionMismatch {
                object,
                theirs,
                ours,
            } => write!(
                f,
                "REFUSED {object} speaks addon protocol {theirs} and this build speaks {ours}. \
                 Nothing it wrote has been read: the version is its own symbol so that a \
                 refusal costs no trust"
            ),
            Self::Ambiguous { class, objects } => write!(
                f,
                "REFUSED {count} libraries of class {class} are installed and opening one of \
                 them would be a choice nobody made: {list}. A class names a role and there is \
                 one library per role (.rulings/ R-200), so remove the one that is not wanted — \
                 or, if this host means to use all of them, enumerate them instead of asking \
                 for the class",
                count = objects.len(),
                list = objects.join(", ")
            ),
        }
    }
}

/// Why a call did not produce an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallFailure {
    /// The request could not be encoded — a `f64` that is not a number.
    Unencodable(String),
    /// The addon returned non-zero: it could not produce even a fault.
    Refused(i32),
    /// The addon returned success and a null or empty buffer.
    EmptyAnswer,
    /// The bytes came back but are not an answer this build can read.
    Unreadable(String),
    /// The answer is an answer, from the wrong protocol version.
    VersionMismatch { theirs: u32, ours: u32 },
}

impl std::fmt::Display for CallFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unencodable(e) => write!(f, "the request could not be encoded: {e}"),
            Self::Refused(code) => write!(
                f,
                "the addon refused the call with code {code} and allocated nothing"
            ),
            Self::EmptyAnswer => write!(f, "the addon reported success and returned no bytes"),
            Self::Unreadable(e) => write!(f, "the addon's answer could not be read: {e}"),
            Self::VersionMismatch { theirs, ours } => write!(
                f,
                "the addon answered in protocol {theirs} and this build speaks {ours}"
            ),
        }
    }
}

impl Loaded {
    /// The class this addon was opened for.
    #[must_use]
    pub fn class(&self) -> &str {
        &self.class
    }

    /// The object's name — its file name, which `cargo rivet library` has
    /// already refused to let differ from its `DT_SONAME`.
    #[must_use]
    pub fn soname(&self) -> &str {
        self.object
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or_default()
    }

    /// The object this addon was loaded from, absolute.
    #[must_use]
    pub fn object(&self) -> &Path {
        &self.object
    }

    /// **Open the addon of this class, if exactly one is installed.**
    ///
    /// The supervisor names what it joined in [`ADDONS_ENV`], so this
    /// enumerates rather than guesses a filename. Three outcomes, and each is
    /// a different thing to put in a journal:
    ///
    /// * **one library of this class** — it is opened, by path;
    /// * **none** — [`OpenFailure::NotLoaded`], the ordinary answer on a
    ///   machine with no addon of this class and not a fault;
    /// * **several** — [`OpenFailure::Ambiguous`], naming every one of them.
    ///   `.rulings/` R-200 makes a class a role and a role one library, so two
    ///   is an operator error rather than a choice for a host to make
    ///   silently. A host that wants all of them calls [`installed`] and
    ///   [`Loaded::open_path`].
    ///
    /// # The fallback, and why it is still here
    ///
    /// **When [`ADDONS_ENV`] is absent entirely, this derives the old name**
    /// — `lib<class>.so.1`, [`abi::soname_for_class`] — and asks the loader
    /// for it, exactly as it did before 2026-09-19. That is not nostalgia: an
    /// application is installed independently of the image that supervises it,
    /// so a host built today runs on a board running an older system, and on
    /// that board the variable is not set. The fallback is what makes such a
    /// host find its addon anyway, and it disappears from the story the moment
    /// that board takes a system update.
    ///
    /// **An absent variable and an empty one are not the same thing**, which
    /// is why `rivet_core::service::addon_env` returns `None` rather than an
    /// empty string: absent means *this machine does not name addons*, and
    /// empty would mean *it names none*, and only the first may fall back.
    ///
    /// # Errors
    /// [`OpenFailure`], each variant a different thing to say in a journal.
    pub fn open(class: &str) -> Result<Self, OpenFailure> {
        let Ok(value) = std::env::var(ADDONS_ENV) else {
            return Self::open_derived(class);
        };
        let mut matches: Vec<Installed> = joined(&value)
            .into_iter()
            .filter(|entry| entry.class == class)
            .collect();
        match matches.len() {
            0 => Err(OpenFailure::NotLoaded {
                looked_for: class.to_string(),
                reason: "this machine named the libraries it joined and none of them is of this \
                         class"
                    .to_string(),
            }),
            1 => {
                let only = matches.remove(0);
                let mut loaded = Self::open_path(&only.path)?;
                // The class the supervisor matched on, carried through so
                // that `class()` answers the same thing it answered before
                // this channel existed.
                loaded.class = only.class;
                Ok(loaded)
            }
            _ => Err(OpenFailure::Ambiguous {
                class: class.to_string(),
                objects: matches
                    .iter()
                    .map(|entry| entry.path.display().to_string())
                    .collect(),
            }),
        }
    }

    /// **Open a named object**, with the same three-symbol handshake
    /// [`Loaded::open`] does.
    ///
    /// This is what a host calls when it means to use **every** library of a
    /// class rather than the one of them — a renderer drawing through each
    /// installed background pack — having got the paths from [`installed`].
    ///
    /// **[`Loaded::class`] is empty for an addon opened this way**, and
    /// deliberately: nothing about a path says what class the supervisor
    /// matched on, and inventing one from a directory name would be this crate
    /// deriving a fact it was not told — the habit the enumeration channel
    /// exists to end. A caller that knows the class already knows it; one that
    /// wants the addon's own answer asks it to [`Request::Describe`] itself,
    /// which is what a host should compare against its expectation anyway.
    ///
    /// # Errors
    /// [`OpenFailure`]. [`OpenFailure::ClassNotNameable`] and
    /// [`OpenFailure::Ambiguous`] cannot arise here — there is no class to
    /// spell and nothing to choose between.
    pub fn open_path(object: &Path) -> Result<Self, OpenFailure> {
        let shown = object.display().to_string();
        let c_path = CString::new(shown.as_bytes()).map_err(|_| OpenFailure::NotLoaded {
            looked_for: shown.clone(),
            reason: "the path has a NUL byte in it, so the loader cannot be asked for it"
                .to_string(),
        })?;
        Self::dlopen(&c_path, shown, String::new())
    }

    /// The old lookup, kept for a host running on an image that predates the
    /// enumeration channel — see [`Loaded::open`]'s account of the fallback.
    fn open_derived(class: &str) -> Result<Self, OpenFailure> {
        if !abi::class_is_nameable(class) {
            return Err(OpenFailure::ClassNotNameable {
                class: class.to_string(),
            });
        }
        let soname = abi::soname_for_class(class);
        let c_soname = CString::new(soname.clone()).map_err(|_| OpenFailure::ClassNotNameable {
            class: class.to_string(),
        })?;
        Self::dlopen(&c_soname, soname, class.to_string())
    }

    /// **The handshake, once.** Both ways in reach the loader through here, so
    /// a named object and a derived name agree on what makes an object an
    /// addon rather than agreeing today and drifting later (`ADR-011`).
    ///
    /// `class` is what the caller already knows; empty when it does not, in
    /// which case the object's own file name stands in until the addon is
    /// asked to describe itself.
    fn dlopen(name: &CString, shown: String, class: String) -> Result<Self, OpenFailure> {
        // SAFETY: `name` is a valid NUL-terminated string for the length of
        // this call. RTLD_LOCAL keeps the addon's symbols out of the global
        // namespace, so two addons exporting the same three names do not
        // resolve into each other — which they would under RTLD_GLOBAL,
        // silently, and only on a machine with two addons installed. That
        // machine is no longer hypothetical: `.rulings/` R-199 D1 makes two
        // background packs the case to support.
        let handle = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if handle.is_null() {
            return Err(OpenFailure::NotLoaded {
                looked_for: shown,
                reason: last_dlerror(),
            });
        }

        let version: VersionFn = unsafe { symbol(handle, abi::VERSION_SYMBOL) }.ok_or_else(|| {
            OpenFailure::NotAnAddon {
                object: shown.clone(),
                symbol: symbol_name(abi::VERSION_SYMBOL),
            }
        })?;
        // The handshake, before a single byte either side wrote is parsed.
        let theirs = unsafe { version() };
        if theirs != abi::VERSION {
            return Err(OpenFailure::VersionMismatch {
                object: shown,
                theirs,
                ours: abi::VERSION,
            });
        }

        let call: CallFn = unsafe { symbol(handle, abi::CALL_SYMBOL) }.ok_or_else(|| {
            OpenFailure::NotAnAddon {
                object: shown.clone(),
                symbol: symbol_name(abi::CALL_SYMBOL),
            }
        })?;
        let free: FreeFn = unsafe { symbol(handle, abi::FREE_SYMBOL) }.ok_or_else(|| {
            OpenFailure::NotAnAddon {
                object: shown.clone(),
                symbol: symbol_name(abi::FREE_SYMBOL),
            }
        })?;

        // **The drawing half is optional, and it is all or nothing.** A
        // reading source exports neither of these and is complete; a pack
        // exports both. One without the other is an object that says it draws
        // and cannot be asked what layout it draws in, or the reverse — either
        // way it was built by a macro nobody has, so it is refused by name
        // rather than treated as the half that happens to be there.
        let draw: Option<DrawFn> = unsafe { symbol(handle, frame::DRAW_SYMBOL) };
        let frame_version: Option<VersionFn> =
            unsafe { symbol(handle, frame::FRAME_VERSION_SYMBOL) };
        let frame_abi = match (draw, frame_version) {
            (None, None) => None,
            (Some(_), Some(version)) => {
                // SAFETY: the symbol is this crate's own constant and its
                // signature is fixed by `frame::FRAME_VERSION_SYMBOL`'s
                // documented type.
                Some(unsafe { version() })
            }
            (Some(_), None) => {
                return Err(OpenFailure::NotAnAddon {
                    object: shown,
                    symbol: symbol_name(frame::FRAME_VERSION_SYMBOL),
                });
            }
            (None, Some(_)) => {
                return Err(OpenFailure::NotAnAddon {
                    object: shown,
                    symbol: symbol_name(frame::DRAW_SYMBOL),
                });
            }
        };

        // **The GL half is optional too, and all-or-nothing on the same
        // terms.** A pack with no GPU path exports neither symbol and is
        // complete; one with a GPU path exports both. One without the other
        // was built by a macro nobody has.
        let draw_gl: Option<DrawGlFn> = unsafe { symbol(handle, gl::DRAW_GL_SYMBOL) };
        let gl_version: Option<VersionFn> = unsafe { symbol(handle, gl::GL_VERSION_SYMBOL) };
        let gl_abi = match (draw_gl, gl_version) {
            (None, None) => None,
            // SAFETY: the symbol is this crate's own constant and its
            // signature is fixed by `gl::GL_VERSION_SYMBOL`'s documented type.
            (Some(_), Some(version)) => Some(unsafe { version() }),
            (Some(_), None) => {
                return Err(OpenFailure::NotAnAddon {
                    object: shown,
                    symbol: symbol_name(gl::GL_VERSION_SYMBOL),
                });
            }
            (None, Some(_)) => {
                return Err(OpenFailure::NotAnAddon {
                    object: shown,
                    symbol: symbol_name(gl::DRAW_GL_SYMBOL),
                });
            }
        };

        Ok(Self {
            class,
            object: PathBuf::from(shown),
            call,
            free,
            draw,
            frame_abi,
            draw_gl,
            gl_abi,
        })
    }

    /// **Ask the addon one question.**
    ///
    /// Blocks until it answers. See this module's own note on containing a
    /// hang: the caller runs this off whatever loop keeps it alive.
    ///
    /// # Errors
    /// [`CallFailure`] when no answer could be read. A [`Response::Fault`] is
    /// **not** an error here — the addon answered, and what it said was that
    /// it could not do the thing.
    pub fn call(&self, request: &Request) -> Result<Response, CallFailure> {
        let encoded =
            crate::protocol::encode(request).map_err(|e| CallFailure::Unencodable(e.to_string()))?;

        let mut out: *mut u8 = std::ptr::null_mut();
        let mut out_len: usize = 0;

        // SAFETY: `encoded` outlives the call; the addon copies on entry
        // (abi rule 1). `out` and `out_len` are ours and writable.
        let code = unsafe {
            (self.call)(
                encoded.as_ptr(),
                encoded.len(),
                &raw mut out,
                &raw mut out_len,
            )
        };
        if code != 0 {
            return Err(CallFailure::Refused(code));
        }
        if out.is_null() || out_len == 0 {
            return Err(CallFailure::EmptyAnswer);
        }

        // SAFETY: the addon reported success, which by abi rule 2 means `out`
        // holds `out_len` bytes it allocated. Copied at once so the buffer is
        // released on every path below, including the error ones.
        let bytes = unsafe { std::slice::from_raw_parts(out, out_len) }.to_vec();
        // SAFETY: handed back exactly what was received, once, to the side
        // that allocated it.
        unsafe { (self.free)(out, out_len) };

        let envelope: Envelope<Response> = crate::protocol::decode(&bytes)
            .map_err(|e| CallFailure::Unreadable(e.to_string()))?;
        if envelope.v != abi::VERSION {
            return Err(CallFailure::VersionMismatch {
                theirs: envelope.v,
                ours: abi::VERSION,
            });
        }
        Ok(envelope.body)
    }

    /// **Whether this object draws** — whether it exports
    /// [`frame::DRAW_SYMBOL`].
    ///
    /// A host that means to draw asks this rather than inferring it from a
    /// class or a name, because the symbol is a property of the **object**
    /// while a class and a name are properties of what somebody wrote in a
    /// manifest.
    #[must_use]
    pub fn draws(&self) -> bool {
        self.draw.is_some()
    }

    /// The frame descriptor version this object speaks, or `None` when it
    /// does not draw. See the field's own note on why a disagreement is not a
    /// refusal to load.
    #[must_use]
    pub fn frame_abi(&self) -> Option<u32> {
        self.frame_abi
    }

    /// **Whether this object can draw with the GPU** — whether it exports
    /// [`gl::DRAW_GL_SYMBOL`].
    ///
    /// [`Loaded::draws`]'s argument applies unchanged: the symbol is a
    /// property of the object, and a class or a name is a property of what
    /// somebody wrote in a manifest.
    ///
    /// **This is not the same question as whether a given background draws
    /// with the GPU.** That one is [`crate::protocol::Background::drawn`],
    /// and R-207's pack answers the two differently on purpose — the object
    /// has a GPU path, and two of its five backgrounds do not use it.
    #[must_use]
    pub fn draws_gl(&self) -> bool {
        self.draw_gl.is_some()
    }

    /// The GL descriptor version this object speaks, or `None` when it has no
    /// GPU path.
    #[must_use]
    pub fn gl_abi(&self) -> Option<u32> {
        self.gl_abi
    }

    /// **Hand the addon one frame, to draw into a framebuffer the host has
    /// already bound.**
    ///
    /// # What the CALLER must have done first
    ///
    /// This function checks the descriptor and nothing about GL, because
    /// nothing about GL is visible from here. Before calling it the host must
    /// have made its EGL context current on **this** thread, created and
    /// bound the target, and set the viewport to `width` by `height`.
    /// `rivet_sdk::gpu` is what does that in this project, and
    /// [`crate::gl`]'s header is the contract in full.
    ///
    /// Everything this *can* check, it checks before the call crosses: that
    /// the object has a GPU path, that it speaks this build's GL descriptor
    /// version, that the geometry is a geometry, and that both ids fit the
    /// fixed-width fields.
    ///
    /// # Errors
    /// [`DrawFailure`] — [`DrawFailure::DoesNotDrawGl`] where the CPU path
    /// would say [`DrawFailure::DoesNotDraw`], and otherwise the same
    /// vocabulary, because a refusal means the same thing whichever boundary
    /// carried it.
    pub fn draw_gl(
        &self,
        target: &gl::GlTarget,
        scene: &frame::Scene<'_>,
    ) -> Result<(), DrawFailure> {
        self.draw_gl_claiming_abi(gl::GL_VERSION, target, scene)
    }

    /// **[`Loaded::draw_gl`], with the descriptor's version written by the
    /// caller.**
    ///
    /// [`Loaded::draw_claiming_abi`]'s twin and its reason: the refusal an
    /// addon owes a mismatched host should be provable from the outside, by a
    /// probe, on a board — not only by a unit test inside one build where
    /// both halves are the same code.
    ///
    /// An ordinary host calls [`Loaded::draw_gl`] and never this.
    ///
    /// # Errors
    /// [`DrawFailure`], as [`Loaded::draw_gl`].
    pub fn draw_gl_claiming_abi(
        &self,
        abi_claim: u32,
        target: &gl::GlTarget,
        scene: &frame::Scene<'_>,
    ) -> Result<(), DrawFailure> {
        let Some(draw_gl) = self.draw_gl else {
            return Err(DrawFailure::DoesNotDrawGl {
                object: self.soname().to_string(),
            });
        };
        if self.gl_abi != Some(gl::GL_VERSION) {
            return Err(DrawFailure::GlAbiMismatch {
                object: self.soname().to_string(),
                theirs: self.gl_abi.unwrap_or_default(),
                ours: gl::GL_VERSION,
            });
        }

        let background =
            frame::id_bytes(scene.background).ok_or_else(|| DrawFailure::IdNotCarryable {
                which: "background",
                id: scene.background.to_string(),
            })?;
        let theme = frame::id_bytes(scene.theme).ok_or_else(|| DrawFailure::IdNotCarryable {
            which: "theme",
            id: scene.theme.to_string(),
        })?;

        let descriptor = gl::GlFrame {
            abi: abi_claim,
            width: target.width(),
            height: target.height(),
            framebuffer: target.framebuffer(),
            reserved: 0,
            reserved2: 0,
            time_ms: scene.time_ms,
            background,
            theme,
        };
        // SAFETY: the descriptor lives on this stack frame for the length of
        // the call, and it is the only thing crossing — there is no buffer
        // here whose size the addon has to trust, which is the whole reason
        // this path has fewer checks than `draw_claiming_abi`.
        let code = unsafe { draw_gl(&raw const descriptor) };
        if code == frame::DREW {
            Ok(())
        } else {
            Err(DrawFailure::Refused {
                code,
                object: self.soname().to_string(),
            })
        }
    }

    /// **Hand the addon one frame of the host's own surface.**
    ///
    /// Blocks until the addon returns, on the caller's thread, with no
    /// allocation on either side — [`crate::frame`]'s header is why the error
    /// path carries a code rather than a sentence.
    ///
    /// Everything this checks, it checks **before** a pointer crosses: that
    /// the object draws at all, that it speaks this build's descriptor
    /// version, that the buffer really is `stride * height` bytes, and that
    /// both ids are ones the descriptor can carry. An addon receiving a
    /// descriptor from this function is receiving one whose every field has
    /// already been agreed.
    ///
    /// # Errors
    /// [`DrawFailure`], each variant a different thing to say in a journal —
    /// and a host redrawing thirty times a second says it once and counts the
    /// rest.
    pub fn draw(
        &self,
        target: &mut Target<'_>,
        scene: &frame::Scene<'_>,
    ) -> Result<(), DrawFailure> {
        self.draw_claiming_abi(frame::FRAME_VERSION, target, scene)
    }

    /// **[`Loaded::draw`], with the descriptor's version written by the
    /// caller.**
    ///
    /// This exists so that the refusal an addon owes a mismatched host can be
    /// proven **from the outside**, by a probe, on a board — rather than only
    /// by a unit test inside one build where both halves are the same code.
    /// `examples/probes/frame-probe-host` is what calls it, and it hands over
    /// a version nothing speaks and then reads the surface back to show that
    /// no byte of it moved.
    ///
    /// An ordinary host calls [`Loaded::draw`] and never this.
    ///
    /// # Errors
    /// [`DrawFailure`], as [`Loaded::draw`].
    pub fn draw_claiming_abi(
        &self,
        abi_claim: u32,
        target: &mut Target<'_>,
        scene: &frame::Scene<'_>,
    ) -> Result<(), DrawFailure> {
        let Some(draw) = self.draw else {
            return Err(DrawFailure::DoesNotDraw {
                object: self.soname().to_string(),
            });
        };
        // The version this build speaks against the version the OBJECT
        // answered — asked of the symbol, before a descriptor exists. A
        // caller deliberately claiming a different one in `abi_claim` is
        // exercising the addon's own check and not this one.
        if self.frame_abi != Some(frame::FRAME_VERSION) {
            return Err(DrawFailure::FrameAbiMismatch {
                object: self.soname().to_string(),
                theirs: self.frame_abi.unwrap_or_default(),
                ours: frame::FRAME_VERSION,
            });
        }

        let background =
            frame::id_bytes(scene.background).ok_or_else(|| DrawFailure::IdNotCarryable {
                which: "background",
                id: scene.background.to_string(),
            })?;
        let theme = frame::id_bytes(scene.theme).ok_or_else(|| DrawFailure::IdNotCarryable {
            which: "theme",
            id: scene.theme.to_string(),
        })?;

        let descriptor = frame::Frame {
            abi: abi_claim,
            width: target.width,
            height: target.height,
            stride: target.stride,
            format: target.format,
            reserved: 0,
            pixels: target.pixels.as_mut_ptr(),
            time_ms: scene.time_ms,
            background,
            theme,
        };

        // SAFETY: `Target::new` established that `pixels` is at least
        // `stride * height` bytes, `descriptor` lives for the length of this
        // call, and `target` is borrowed mutably so nothing else touches the
        // surface while the addon has it.
        let code = unsafe { draw(std::ptr::from_ref(&descriptor)) };
        if code == frame::DREW {
            Ok(())
        } else {
            Err(DrawFailure::Refused {
                code,
                object: self.soname().to_string(),
            })
        }
    }
}

/// **The host's surface, on the host's side of the call.**
///
/// It is a borrow and a shape, and nothing else: the buffer belongs to
/// whatever allocated it — a `Vec` in a probe, a mapped dumb buffer in a
/// renderer — and this type never owns, resizes or frees it. That is the same
/// sentence [`crate::frame`]'s header says to the addon, said to the host.
///
/// # Why a constructor rather than public fields
///
/// The addon **trusts** the descriptor: given a `stride` and a `height` it
/// writes that many bytes, because it has no way to ask how large the
/// allocation really is. So `stride * height` fitting inside the buffer is an
/// invariant that has to hold at the moment the descriptor is composed, and
/// the only way to make that certain is to establish it once, here, where the
/// slice and the numbers are in the same place.
pub struct Target<'a> {
    pixels: &'a mut [u8],
    width: u32,
    height: u32,
    stride: u32,
    format: u32,
}

impl<'a> Target<'a> {
    /// Describe a buffer the host owns.
    ///
    /// # Errors
    /// [`DrawFailure::Geometry`] when the numbers do not describe a surface,
    /// and [`DrawFailure::SurfaceTooSmall`] when they describe one larger than
    /// the slice.
    pub fn new(
        pixels: &'a mut [u8],
        width: u32,
        height: u32,
        stride: u32,
        format: u32,
    ) -> Result<Self, DrawFailure> {
        if width == 0 || height == 0 {
            return Err(DrawFailure::Geometry {
                why: "a surface with no width or no height is not one to draw into",
            });
        }
        if format != frame::FORMAT_XRGB8888 {
            return Err(DrawFailure::Geometry {
                why: "the only pixel format this build composes is XRGB8888",
            });
        }
        let row = (width as usize)
            .checked_mul(4)
            .ok_or(DrawFailure::Geometry {
                why: "a row of this width cannot be addressed",
            })?;
        if (stride as usize) < row {
            return Err(DrawFailure::Geometry {
                why: "the stride is narrower than one row, so row one would overwrite row two",
            });
        }
        let need = (stride as usize)
            .checked_mul(height as usize)
            .ok_or(DrawFailure::Geometry {
                why: "a surface of this size cannot be addressed",
            })?;
        if pixels.len() < need {
            return Err(DrawFailure::SurfaceTooSmall {
                need,
                have: pixels.len(),
            });
        }
        Ok(Self {
            pixels,
            width,
            height,
            stride,
            format,
        })
    }

    /// **The bytes, to read back.** This is what a probe asserts on after a
    /// frame, and what a renderer blits.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        self.pixels
    }

    /// The bytes, to prepare. A host fills its own padding, clears the surface
    /// or writes a sentinel through here before it hands the frame over.
    pub fn pixels_mut(&mut self) -> &mut [u8] {
        self.pixels
    }

    /// One pixel, read back as `0x00RR_GGBB`, or `None` outside the visible
    /// area.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<u32> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let at = (y as usize) * (self.stride as usize) + (x as usize) * 4;
        let bytes: [u8; 4] = self.pixels.get(at..at + 4)?.try_into().ok()?;
        Some(u32::from_le_bytes(bytes))
    }
}

/// Why a frame did not get drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrawFailure {
    /// The object exports no [`frame::DRAW_SYMBOL`]: it is a reading source,
    /// and this is the ordinary answer when a host asks the wrong addon to
    /// draw rather than a fault in either of them.
    DoesNotDraw { object: String },
    /// The object draws, in a descriptor layout this build does not speak.
    /// **Read from the object's own symbol, so nothing was composed and no
    /// pointer crossed.**
    FrameAbiMismatch {
        object: String,
        theirs: u32,
        ours: u32,
    },
    /// The object exports no [`gl::DRAW_GL_SYMBOL`]: it has no GPU path. Like
    /// [`DrawFailure::DoesNotDraw`] this is an ordinary answer rather than a
    /// fault — a host that can draw either way asks this one first and falls
    /// back to the CPU boundary.
    DoesNotDrawGl { object: String },
    /// The object has a GPU path, in a GL descriptor layout this build does
    /// not speak. Read from the object's own symbol, so nothing crossed.
    GlAbiMismatch {
        object: String,
        theirs: u32,
        ours: u32,
    },
    /// The numbers do not describe a surface.
    Geometry { why: &'static str },
    /// The numbers describe a surface larger than the buffer behind them. The
    /// one check the addon cannot make for itself.
    SurfaceTooSmall { need: usize, have: usize },
    /// An id the fixed-width field cannot carry — see [`frame::ID_BYTES`].
    IdNotCarryable { which: &'static str, id: String },
    /// **The addon refused the frame**, with one of [`crate::frame`]'s codes.
    /// The sentence is composed here because no sentence crosses a boundary
    /// used thirty times a second.
    Refused { code: i32, object: String },
}

impl DrawFailure {
    /// What [`frame`]'s code means, in a phrase a journal line can carry.
    fn refusal(code: i32) -> &'static str {
        match code {
            frame::REFUSED_VERSION => {
                "the addon read the descriptor's version and stopped there, so no pixel was \
                 written"
            }
            frame::REFUSED_DESCRIPTOR => {
                "the addon would not believe the descriptor's own shape — a null surface, a \
                 zero dimension, or a stride that does not hold a row"
            }
            frame::REFUSED_FORMAT => "the addon does not write this pixel format",
            frame::REFUSED_BACKGROUND => "this addon has no background of that id",
            frame::REFUSED_THEME => "that background has no theme of that id",
            frame::REFUSED_SURFACE => {
                "the addon will not draw into a surface of this shape, which is its judgement \
                 about a surface the boundary found perfectly well described"
            }
            frame::PANICKED => {
                "the addon panicked while drawing and the boundary caught it, so this frame is \
                 lost and the host is not"
            }
            _ => "the addon answered with a code this build has no name for",
        }
    }
}

impl std::fmt::Display for DrawFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DoesNotDraw { object } => write!(
                f,
                "{object} exports no drawing entry, so it is a source that measures rather than \
                 one that draws. This is the ordinary answer for a reading addon and not a fault"
            ),
            Self::FrameAbiMismatch {
                object,
                theirs,
                ours,
            } => write!(
                f,
                "REFUSED {object} draws in frame descriptor layout {theirs} and this build \
                 composes {ours}. The version came from its own symbol, so nothing has been \
                 composed and no pointer has crossed"
            ),
            Self::DoesNotDrawGl { object } => write!(
                f,
                "{object} exports no GPU drawing entry, so it draws with the CPU or not at all. \
                 This is the ordinary answer for a pack built before R-206 and not a fault — a \
                 host that can do both asks this first and falls back"
            ),
            Self::GlAbiMismatch {
                object,
                theirs,
                ours,
            } => write!(
                f,
                "REFUSED {object} draws on the GPU in descriptor layout {theirs} and this build \
                 composes {ours}. The version came from its own symbol, so nothing has been \
                 composed and no descriptor has crossed"
            ),
            Self::Geometry { why } => write!(f, "REFUSED the surface was not described: {why}"),
            Self::SurfaceTooSmall { need, have } => write!(
                f,
                "REFUSED the descriptor would tell the addon it may write {need} bytes and the \
                 buffer behind it holds {have}. An addon trusts stride and height, so this is \
                 the one bound it cannot check for itself"
            ),
            Self::IdNotCarryable { which, id } => write!(
                f,
                "REFUSED the {which} id {id:?} is not one the frame descriptor can carry — \
                 lower-case letters, digits and a dash, not starting or ending with one, and \
                 at most {max} bytes",
                max = frame::ID_BYTES
            ),
            Self::Refused { code, object } => write!(
                f,
                "{object} did not draw this frame (code {code}): {why}",
                why = Self::refusal(*code)
            ),
        }
    }
}

/// `dlsym`, transmuted to a function pointer.
///
/// # Safety
/// `handle` must be a live `dlopen` handle and `name` a NUL-terminated symbol
/// whose real signature is `F`. Both are satisfied by the three constants in
/// [`crate::abi`], which is why this is private and takes no other caller.
unsafe fn symbol<F: Copy>(handle: *mut libc::c_void, name: &[u8]) -> Option<F> {
    // SAFETY: the caller's contract, plus `name` being one of this crate's
    // own constants, each already NUL-terminated.
    let address = unsafe { libc::dlsym(handle, name.as_ptr().cast::<libc::c_char>()) };
    if address.is_null() {
        return None;
    }
    assert_eq!(
        std::mem::size_of::<F>(),
        std::mem::size_of::<*mut libc::c_void>(),
        "a function pointer is one word on every target this runs on"
    );
    // SAFETY: `address` is non-null and, by the caller's contract, is the
    // symbol whose signature is `F`.
    Some(unsafe { std::mem::transmute_copy::<*mut libc::c_void, F>(&address) })
}

fn symbol_name(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().saturating_sub(1)]).into_owned()
}

fn last_dlerror() -> String {
    // SAFETY: `dlerror` returns a pointer to a static buffer or null, and is
    // read immediately, before anything else on this thread can call into the
    // loader again.
    let message = unsafe { libc::dlerror() };
    if message.is_null() {
        return "the loader gave no reason".to_string();
    }
    unsafe { std::ffi::CStr::from_ptr(message) }
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Every test that touches `RIVET_ADDONS` takes this lock.** The
    /// environment is per-process and `cargo test` is per-thread, so two tests
    /// setting the same variable would race — and the failure would be
    /// intermittent, which is the worst kind to leave in a suite. Rust makes
    /// `set_var` `unsafe` for the same reason.
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Run `body` with [`ADDONS_ENV`] set to `value`, or unset for `None`, and
    /// put it back afterwards.
    fn with_addons<T>(value: Option<&str>, body: impl FnOnce() -> T) -> T {
        let _guard = ENV.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let before = std::env::var(ADDONS_ENV).ok();
        // SAFETY: the lock above makes this the only thread touching this
        // variable for the length of the call, which is the whole of what
        // `set_var`'s contract asks for.
        unsafe {
            match value {
                Some(v) => std::env::set_var(ADDONS_ENV, v),
                None => std::env::remove_var(ADDONS_ENV),
            }
        }
        let out = body();
        // SAFETY: as above, and the lock is still held.
        unsafe {
            match before {
                Some(v) => std::env::set_var(ADDONS_ENV, v),
                None => std::env::remove_var(ADDONS_ENV),
            }
        }
        out
    }

    #[test]
    fn a_class_that_cannot_be_a_filename_is_refused_before_the_loader_is_asked() {
        // On the fallback path, where a class still becomes a filename and so
        // still has to be one.
        with_addons(None, || match Loaded::open("../../etc/passwd") {
            Err(OpenFailure::ClassNotNameable { class }) => assert!(class.contains("passwd")),
            other => panic!("read as {other:?}", other = other.map(|l| l.class)),
        });
    }

    /// **A host built today still finds its addon on a board running an older
    /// image**, where nothing sets `RIVET_ADDONS`. The name it asks for is the
    /// one that image's supervisor arranged for it to find.
    #[test]
    fn an_absent_variable_falls_back_to_the_name_an_older_image_arranged() {
        with_addons(None, || {
            match Loaded::open("no-such-class-is-installed") {
                Err(OpenFailure::NotLoaded { looked_for, .. }) => {
                    assert_eq!(looked_for, "libno-such-class-is-installed.so.1");
                }
                other => panic!("read as {other:?}", other = other.map(|l| l.class)),
            }
        });
    }

    /// **An empty variable is not an absent one.** The machine named its joins
    /// and there were none, so there is nothing to fall back to — and the
    /// refusal says the class rather than a filename nobody will find.
    #[test]
    fn a_machine_that_named_no_library_of_this_class_does_not_fall_back() {
        with_addons(Some("other-class=/state/lib/x/lib/libother.so.1"), || {
            match Loaded::open("probe-source") {
                Err(OpenFailure::NotLoaded { looked_for, reason }) => {
                    assert_eq!(looked_for, "probe-source");
                    assert!(reason.contains("named"), "{reason}");
                }
                other => panic!("read as {other:?}", other = other.map(|l| l.class)),
            }
        });
    }

    /// **The case the naming rule could not express, refused rather than
    /// guessed** — `.rulings/` R-200. Two libraries of one class, and both are
    /// named so the operator knows which two.
    #[test]
    fn two_libraries_of_one_class_are_refused_and_both_are_named() {
        let value = "probe-source=/state/lib/a/lib/libprobe-source.so.1:\
                     probe-source=/state/lib/b/lib/libprobe-source-b.so.1";
        with_addons(Some(value), || match Loaded::open("probe-source") {
            Err(OpenFailure::Ambiguous { class, objects }) => {
                assert_eq!(class, "probe-source");
                assert_eq!(objects.len(), 2);
                let sentence = OpenFailure::Ambiguous { class, objects }.to_string();
                assert!(sentence.contains("libprobe-source.so.1"), "{sentence}");
                assert!(sentence.contains("libprobe-source-b.so.1"), "{sentence}");
            }
            other => panic!("read as {other:?}", other = other.map(|l| l.class)),
        });
    }

    /// **Enumeration reports every library of the class, by path**, which is
    /// what a host wanting all of them uses instead of `open`.
    #[test]
    fn enumeration_reports_every_library_of_the_class_and_nothing_else() {
        let value = "aura-background-pack=/state/lib/a/lib/libpackone.so.1:\
                     aura-reading-source=/state/lib/c/lib/libreading.so.1:\
                     aura-background-pack=/state/lib/b/lib/libpacktwo.so.1";
        with_addons(Some(value), || {
            let found = installed("aura-background-pack");
            assert_eq!(found.len(), 2);
            assert_eq!(found[0].soname(), "libpackone.so.1");
            assert_eq!(found[1].soname(), "libpacktwo.so.1");
            assert!(installed("no-such-class").is_empty());
        });
    }

    /// Nothing named at all — an unset variable — is an empty enumeration and
    /// not a panic, because that is what a host on an older image reads.
    #[test]
    fn enumeration_of_an_unnamed_machine_is_empty_rather_than_a_failure() {
        with_addons(None, || assert!(installed("probe-source").is_empty()));
    }

    /// A malformed entry costs itself and nothing else. The supervisor is the
    /// only writer of this string, so a host that refused everything over one
    /// bad entry would be one an unrelated library could disable.
    #[test]
    fn a_malformed_entry_is_skipped_rather_than_taking_the_rest_with_it() {
        let value = "not-an-entry:probe-source=/state/lib/a/lib/libprobe-source.so.1:=/x:empty=";
        let entries = joined(value);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].class, "probe-source");
        assert_eq!(
            entries[0].path,
            PathBuf::from("/state/lib/a/lib/libprobe-source.so.1")
        );
    }

    #[test]
    fn every_open_failure_names_what_it_refused() {
        let cases = [
            OpenFailure::ClassNotNameable {
                class: "Bad".to_string(),
            },
            OpenFailure::NotLoaded {
                looked_for: "libx.so.1".to_string(),
                reason: "not found".to_string(),
            },
            OpenFailure::NotAnAddon {
                object: "libx.so.1".to_string(),
                symbol: "rivet_addon_abi".to_string(),
            },
            OpenFailure::VersionMismatch {
                object: "libx.so.1".to_string(),
                theirs: 2,
                ours: 1,
            },
            OpenFailure::Ambiguous {
                class: "probe-source".to_string(),
                objects: vec!["/a/libx.so.1".to_string(), "/b/liby.so.1".to_string()],
            },
        ];
        for case in cases {
            let sentence = case.to_string();
            assert!(
                sentence.len() > 40,
                "a refusal is a sentence, not a code: {sentence}"
            );
        }
    }

    /// **The bound an addon cannot check for itself.** It is handed a stride
    /// and a height and writes that many bytes; if the buffer behind them is
    /// smaller, the addon is not at fault and the host's memory is still
    /// corrupt. So the composing side refuses.
    #[test]
    fn a_surface_smaller_than_its_own_geometry_never_becomes_a_descriptor() {
        let mut small = vec![0u8; 100];
        match Target::new(&mut small, 8, 4, 40, frame::FORMAT_XRGB8888) {
            Err(DrawFailure::SurfaceTooSmall { need, have }) => {
                assert_eq!(need, 160);
                assert_eq!(have, 100);
            }
            Err(other) => panic!("read as {other}"),
            Ok(_) => panic!("a surface smaller than its own geometry was accepted"),
        }

        let mut enough = vec![0u8; 160];
        assert!(Target::new(&mut enough, 8, 4, 40, frame::FORMAT_XRGB8888).is_ok());
    }

    #[test]
    fn geometry_that_is_not_a_surface_is_refused_where_it_is_composed() {
        let mut bytes = vec![0u8; 4096];
        let cases: [(u32, u32, u32, u32); 4] = [
            (0, 4, 40, frame::FORMAT_XRGB8888),
            (8, 0, 40, frame::FORMAT_XRGB8888),
            (8, 4, 8, frame::FORMAT_XRGB8888),
            (8, 4, 40, 0),
        ];
        for (width, height, stride, format) in cases {
            let outcome = Target::new(&mut bytes, width, height, stride, format);
            assert!(
                matches!(outcome, Err(DrawFailure::Geometry { .. })),
                "{width}x{height} stride {stride} format {format} should not describe a surface"
            );
        }
    }

    #[test]
    fn a_target_reads_its_own_pixels_back_and_knows_where_the_edge_is() {
        let mut bytes = vec![0u8; 160];
        let mut target =
            Target::new(&mut bytes, 8, 4, 40, frame::FORMAT_XRGB8888).expect("a surface");
        target.pixels_mut()[..4].copy_from_slice(&0x0012_3456u32.to_le_bytes());
        assert_eq!(target.pixel(0, 0), Some(0x0012_3456));
        assert_eq!(target.pixel(7, 3), Some(0));
        assert_eq!(target.pixel(8, 0), None);
        assert_eq!(target.pixel(0, 4), None);
    }

    /// Every refusal is a sentence somebody can act on, including the codes
    /// that come back from the other side of the boundary with no prose
    /// attached — which is the whole reason the prose lives here.
    #[test]
    fn every_draw_failure_names_what_it_refused() {
        let cases = [
            DrawFailure::DoesNotDraw {
                object: "libaura-reading-source.so.1".to_string(),
            },
            DrawFailure::FrameAbiMismatch {
                object: "libpack.so.1".to_string(),
                theirs: 2,
                ours: 1,
            },
            DrawFailure::Geometry { why: "a reason" },
            DrawFailure::SurfaceTooSmall {
                need: 8_294_400,
                have: 1_024,
            },
            DrawFailure::IdNotCarryable {
                which: "theme",
                id: "Ice Blue".to_string(),
            },
            DrawFailure::Refused {
                code: frame::REFUSED_VERSION,
                object: "libpack.so.1".to_string(),
            },
            DrawFailure::Refused {
                code: 9_999,
                object: "libpack.so.1".to_string(),
            },
        ];
        for case in cases {
            let sentence = case.to_string();
            assert!(
                sentence.len() > 40,
                "a refusal is a sentence, not a code: {sentence}"
            );
        }
        assert!(
            DrawFailure::Refused {
                code: frame::REFUSED_VERSION,
                object: "libpack.so.1".to_string(),
            }
            .to_string()
            .contains("no pixel was written"),
            "the version refusal says the thing a reader needs to know"
        );
    }

    /// **The identity a reinstall changes.** Two references under
    /// `/state/lib` for the same class differ in their directory name, which
    /// is the whole of what [`crate::trial`] needs to tell an upgrade apart
    /// from the crash that preceded it.
    #[test]
    fn identity_is_the_reference_directory_and_changes_on_reinstall() {
        let before = Installed {
            class: "probe-source".to_string(),
            path: PathBuf::from("/state/lib/probe-source-1.0.0-aaaa/lib/libprobe-source.so.1"),
        };
        let after = Installed {
            class: "probe-source".to_string(),
            path: PathBuf::from("/state/lib/probe-source-1.0.1-bbbb/lib/libprobe-source.so.1"),
        };
        assert_eq!(before.identity(), "probe-source-1.0.0-aaaa");
        assert_eq!(after.identity(), "probe-source-1.0.1-bbbb");
        assert_ne!(before.identity(), after.identity());
    }

    /// A path that is not laid out under `/state/lib` still yields something
    /// — the absolute path itself — rather than nothing at all.
    #[test]
    fn identity_falls_back_to_the_whole_path_outside_state_lib() {
        let installed = Installed {
            class: "probe-source".to_string(),
            path: PathBuf::from("/tmp/libprobe-source.so.1"),
        };
        assert_eq!(installed.identity(), "/tmp/libprobe-source.so.1");
    }

    /// Opening a path that is not there fails as a loader failure naming the
    /// path, rather than as anything about a class.
    #[test]
    fn a_named_object_that_is_not_there_names_the_path_it_looked_for() {
        match Loaded::open_path(Path::new("/no/such/object/libnope.so.1")) {
            Err(OpenFailure::NotLoaded { looked_for, .. }) => {
                assert!(looked_for.contains("libnope.so.1"), "{looked_for}");
            }
            other => panic!("read as {other:?}", other = other.map(|l| l.class)),
        }
    }
}
