//! **Can the GPU's own buffer reach the glass?** — the piece `mesa-demo`
//! deliberately stopped short of.
//!
//! `examples/probes/mesa-demo` proved that this board compiles and runs
//! shaders: it printed `GL_RENDERER = V3D 4.2`, which is the one line that
//! distinguishes the hardware from Mesa's `llvmpipe` software rasteriser. It
//! rendered **off-screen** and copied the result to `/dev/fb0`, and said so:
//!
//! > Rendering off-screen and copying to the framebuffer reuses a path
//! > `kiosk-demo` already proved and isolates the question to *"can this
//! > machine compile and run a shader"*. It is slower and it tears, and
//! > neither matters for an answer. **Page-flipping is what Aura will need**
//! > and is the next piece, not this one.
//!
//! This is that next piece: of what stands between Aura and the GPU, it is
//! the question that is not ordinary work.
//!
//! # Why a copy is not good enough, stated as arithmetic
//!
//! Aura's background costs 32.12 ms a frame for Contours on `rivet-01`
//! (measured 2026-09-20) against a 33.3 ms budget, and the whole reason to
//! move it to the V3D is that a 96x54 field bilinearly scaled to two megapixels
//! is one textured quad there. **But a GPU that draws fast and then hands the
//! pixels back over the bus has saved nothing**: reading 8.29 MB out of GPU
//! memory costs more than the drawing did. The win exists only if the finished
//! frame is *scanned out of the buffer the GPU drew into*, which is what a GBM
//! surface plus `drmModePageFlip` is.
//!
//! So the question is not "does GL work" — that is answered — but **"can a
//! `gbm_bo` become a DRM framebuffer this CRTC will flip to."**
//!
//! # What this borrows, and from where
//!
//! Nothing here is new ground; it is two proven halves joined.
//!
//! * The EGL bring-up is `mesa-demo`'s, constant for constant, **with one
//!   deliberate difference** — see [`EGL_WINDOW_BIT`], which is the exact
//!   opposite of the lesson that file records.
//! * The KMS enumeration and the flip are `rivet_sdk::screen`'s, whose structs
//!   and `drmModePageFlip` call have driven this board's display for months.
//!   They are copied rather than imported for `mesa-demo`'s own reason: a probe
//!   answering *does this work* should not answer it through an abstraction
//!   that could be the thing that is broken.
//!
//! # No crates
//!
//! `ADR-009` §5, and `kiosk-demo`'s argument that `mesa-demo` repeats: a
//! failure in `gbm-rs` and a failure in `gbm` would look identical from here.
//! The declarations below are the exact set this program calls.

#![allow(unsafe_code)]

use std::ffi::{CString, c_char, c_int, c_uint, c_void};
use std::os::fd::AsRawFd;

// **There is no card node here, and the first draft was wrong to want one.**
//
// `rivet_core::service::device_globs` expands a declared `display` to
// `/dev/dri/renderD*` for a LESSEE and to the card only for the ARBITER — and
// `DisplayRole` is *"not written in a manifest and must not be"*, derived by
// the supervisor, one per machine, which today is `rivet-launcher`. So no
// application can open `/dev/dri/card0`, ever. `ADR-015` §6 arbitrates the
// screen and an application that could ask to be the arbiter could ask for the
// screen.
//
// **A lessee flips anyway**, which is the part that makes a GPU kiosk possible:
// it is handed a DRM *lease* over the socket named by `RIVET_DISPLAY`, and
// `rivet_sdk::screen` says of it — "**A lessee owns the leased objects and may
// flip them** — `drmModePageFlip`". That lease descriptor is a DRM fd. GBM can
// make a device on it and KMS will flip the objects it carries.
//
// So this probe is an ordinary kiosk application that happens to draw with the
// GPU, and that is exactly the thing worth proving.

/// How many frames to flip before reporting.
///
/// **Enough to time, few enough to be over in seconds.** At 60 Hz this is two
/// seconds; the number that matters is the mean interval, which says whether
/// the flips are landing on the vblank or being missed.
const FRAMES: u32 = 120;

/// `DRM_MODE_PAGE_FLIP_EVENT` — `drm_mode.h`. Ask for the completion event,
/// because the point of a flip is knowing when it happened.
const PAGE_FLIP_EVENT: u32 = 0x01;

/// `DRM_MODE_CONNECTED`.
const CONNECTED: c_uint = 1;

/// `GBM_FORMAT_XRGB8888` — `fourcc` for `'X','R','2','4'`.
///
/// **The same format the dumb-buffer path scans out**, so nothing downstream
/// of the flip changes: `rivet_sdk::screen` registers its pages with
/// `drmModeAddFB(.., depth 24, bpp 32, ..)`, which is this.
const GBM_FORMAT_XRGB8888: u32 =
    (b'X' as u32) | ((b'R' as u32) << 8) | ((b'2' as u32) << 16) | ((b'4' as u32) << 24);

/// `GBM_BO_USE_SCANOUT | GBM_BO_USE_RENDERING` — `gbm.h`.
///
/// **Both, and both are load-bearing.** `RENDERING` is what lets EGL draw into
/// it; `SCANOUT` is what lets the display controller read it. A buffer with
/// only the first is the off-screen case `mesa-demo` already did.
const GBM_BO_USE_SCANOUT: u32 = 1 << 0;
const GBM_BO_USE_RENDERING: u32 = 1 << 2;
/// `GBM_BO_USE_LINEAR`, and on this board it is not optional.
///
/// The buffer is allocated by **V3D** and scanned out by **VC4**, which are
/// different devices (see the header). V3D's preferred tiling means nothing to
/// VC4's display FIFO, and a tiled buffer imported across the two shows as a
/// scrambled screen rather than as an error. Linear is the layout both agree
/// on, and it is what every split-SoC compositor asks for at this seam.
const GBM_BO_USE_LINEAR: u32 = 1 << 4;

/// `DRM_FORMAT_XRGB8888` — the same fourcc as [`GBM_FORMAT_XRGB8888`], named
/// twice because one is GBM's word and one is KMS's and they are different
/// headers that happen to agree.
const DRM_FORMAT_XRGB8888: u32 = GBM_FORMAT_XRGB8888;

// ---- EGL, copied from `mesa-demo` -----------------------------------------

type EGLint = i32;
type EGLDisplay = *mut c_void;
type EGLConfig = *mut c_void;
type EGLContext = *mut c_void;
type EGLSurface = *mut c_void;

const EGL_PLATFORM_GBM_KHR: u32 = 0x31D7;
const EGL_NONE: EGLint = 0x3038;
const EGL_OPENGL_ES_API: u32 = 0x30A0;
const EGL_CONTEXT_CLIENT_VERSION: EGLint = 0x3098;
const EGL_SURFACE_TYPE: EGLint = 0x3033;
const EGL_RENDERABLE_TYPE: EGLint = 0x3040;
const EGL_OPENGL_ES2_BIT: EGLint = 0x0004;
const EGL_RED_SIZE: EGLint = 0x3024;
const EGL_GREEN_SIZE: EGLint = 0x3023;
const EGL_BLUE_SIZE: EGLint = 0x3022;
const EGL_ALPHA_SIZE: EGLint = 0x3021;
const EGL_NATIVE_VISUAL_ID: EGLint = 0x302E;

/// **`EGL_WINDOW_BIT`, and `mesa-demo` records the opposite lesson.**
///
/// That program asked for `EGL_PBUFFER_BIT` and the board answered *no ES2
/// configuration*; its note explains why, and the explanation is the reason
/// this constant is right here:
///
/// > a GBM display offers WINDOW configs, because a GBM surface is what one
/// > draws to. This context has no surface at all … so constraining the
/// > surface type asked for a capability nothing here uses.
///
/// This context **does** have a surface, and it is a GBM one. So the config
/// must carry `EGL_WINDOW_BIT`, which is the capability that was there all
/// along. Asking for it is not a repeat of that mistake; it is its mirror.
const EGL_WINDOW_BIT: EGLint = 0x0004;

const GL_RENDERER: u32 = 0x1F01;
const GL_COLOR_BUFFER_BIT: u32 = 0x4000;

#[link(name = "gbm")]
unsafe extern "C" {
    fn gbm_create_device(fd: c_int) -> *mut c_void;
    fn gbm_surface_create(
        gbm: *mut c_void,
        width: u32,
        height: u32,
        format: u32,
        flags: u32,
    ) -> *mut c_void;
    /// **The call this probe exists for.** After `eglSwapBuffers` the finished
    /// buffer is the surface's front; this is how a program gets hold of it so
    /// it can be handed to KMS.
    fn gbm_surface_lock_front_buffer(surface: *mut c_void) -> *mut c_void;
    /// Give it back, or the surface runs out of buffers and
    /// `gbm_surface_lock_front_buffer` starts returning NULL — which is the
    /// leak this probe would otherwise hit within a few frames.
    fn gbm_surface_release_buffer(surface: *mut c_void, bo: *mut c_void);
    /// **Unused, and kept as the thing NOT to reach for.** It returns a GEM
    /// handle on the device that allocated the buffer — V3D — and the CRTC is
    /// VC4's. A handle from one DRM device passed to another names whatever
    /// that device happens to have under the same number, or nothing. This
    /// probe's first working version is the one that learned it.
    #[allow(dead_code)]
    fn gbm_bo_get_handle(bo: *mut c_void) -> u64;
    fn gbm_bo_get_stride(bo: *mut c_void) -> u32;
    /// **Export the buffer as a dma-buf.** The one call that lets a buffer
    /// allocated on V3D be spoken about on VC4 — a descriptor is the only
    /// currency two DRM devices share, because a GEM handle is meaningless
    /// outside the device that issued it.
    fn gbm_bo_get_fd(bo: *mut c_void) -> c_int;
}

#[link(name = "EGL")]
unsafe extern "C" {
    fn eglGetPlatformDisplay(
        platform: u32,
        native: *mut c_void,
        attrib: *const isize,
    ) -> EGLDisplay;
    fn eglInitialize(dpy: EGLDisplay, major: *mut EGLint, minor: *mut EGLint) -> u32;
    fn eglBindAPI(api: u32) -> u32;
    fn eglChooseConfig(
        dpy: EGLDisplay,
        attrib: *const EGLint,
        configs: *mut EGLConfig,
        size: EGLint,
        found: *mut EGLint,
    ) -> u32;
    fn eglGetConfigAttrib(
        dpy: EGLDisplay,
        config: EGLConfig,
        attribute: EGLint,
        value: *mut EGLint,
    ) -> u32;
    fn eglCreateContext(
        dpy: EGLDisplay,
        config: EGLConfig,
        share: EGLContext,
        attrib: *const EGLint,
    ) -> EGLContext;
    fn eglCreateWindowSurface(
        dpy: EGLDisplay,
        config: EGLConfig,
        window: *mut c_void,
        attrib: *const EGLint,
    ) -> EGLSurface;
    fn eglMakeCurrent(
        dpy: EGLDisplay,
        draw: EGLSurface,
        read: EGLSurface,
        ctx: EGLContext,
    ) -> u32;
    fn eglSwapBuffers(dpy: EGLDisplay, surface: EGLSurface) -> u32;
    fn eglGetError() -> EGLint;
}

#[link(name = "GLESv2")]
unsafe extern "C" {
    fn glGetString(name: u32) -> *const c_char;
    fn glClearColor(r: f32, g: f32, b: f32, a: f32);
    fn glClear(mask: u32);
    fn glFinish();
}

// ---- KMS, copied from `rivet_sdk::screen` ---------------------------------

const MODE_NAME_LEN: usize = 32;

#[repr(C)]
struct ModeInfo {
    clock: u32,
    hdisplay: u16,
    hsync_start: u16,
    hsync_end: u16,
    htotal: u16,
    hskew: u16,
    vdisplay: u16,
    vsync_start: u16,
    vsync_end: u16,
    vtotal: u16,
    vscan: u16,
    vrefresh: u32,
    flags: u32,
    kind: u32,
    name: [c_char; MODE_NAME_LEN],
}

#[repr(C)]
struct Resources {
    count_fbs: c_int,
    fbs: *mut u32,
    count_crtcs: c_int,
    crtcs: *mut u32,
    count_connectors: c_int,
    connectors: *mut u32,
    count_encoders: c_int,
    encoders: *mut u32,
    min_width: u32,
    max_width: u32,
    min_height: u32,
    max_height: u32,
}

#[repr(C)]
struct Connector {
    connector_id: u32,
    encoder_id: u32,
    connector_type: u32,
    connector_type_id: u32,
    connection: c_uint,
    mm_width: u32,
    mm_height: u32,
    subpixel: c_uint,
    count_modes: c_int,
    modes: *mut ModeInfo,
    count_props: c_int,
    props: *mut u32,
    prop_values: *mut u64,
    count_encoders: c_int,
    encoders: *mut u32,
}

#[repr(C)]
struct Encoder {
    encoder_id: u32,
    encoder_type: u32,
    crtc_id: u32,
    possible_crtcs: u32,
    possible_clones: u32,
}

#[link(name = "drm")]
unsafe extern "C" {

    fn drmModeGetResources(fd: c_int) -> *mut Resources;
    fn drmModeFreeResources(res: *mut Resources);
    fn drmModeGetConnector(fd: c_int, id: u32) -> *mut Connector;
    fn drmModeFreeConnector(c: *mut Connector);
    fn drmModeGetEncoder(fd: c_int, id: u32) -> *mut Encoder;
    fn drmModeFreeEncoder(e: *mut Encoder);
    fn drmModeAddFB(
        fd: c_int,
        width: u32,
        height: u32,
        depth: u8,
        bpp: u8,
        pitch: u32,
        bo_handle: u32,
        buf_id: *mut u32,
    ) -> c_int;
    fn drmModeRmFB(fd: c_int, buffer_id: u32) -> c_int;
    /// **Import a dma-buf and get a GEM handle for THIS device.** The other
    /// half of `gbm_bo_get_fd`, and the seam the whole probe turns on.
    fn drmPrimeFDToHandle(fd: c_int, prime_fd: c_int, handle: *mut u32) -> c_int;
    /// **`AddFB2`, not `AddFB`.** The older call takes a depth and a bpp and
    /// guesses a format; this one is given the fourcc outright, and a buffer
    /// that crossed a device boundary is exactly the case where guessing is
    /// how a screen ends up scrambled instead of refused.
    fn drmModeAddFB2(
        fd: c_int,
        width: u32,
        height: u32,
        pixel_format: u32,
        bo_handles: *const u32,
        pitches: *const u32,
        offsets: *const u32,
        buf_id: *mut u32,
        flags: u32,
    ) -> c_int;
    fn close(fd: c_int) -> c_int;
    fn drmModeSetCrtc(
        fd: c_int,
        crtc_id: u32,
        buffer_id: u32,
        x: u32,
        y: u32,
        connectors: *mut u32,
        count: c_int,
        mode: *mut ModeInfo,
    ) -> c_int;
    fn drmModePageFlip(
        fd: c_int,
        crtc_id: u32,
        fb_id: u32,
        flags: u32,
        user_data: *mut c_void,
    ) -> c_int;
    fn __errno_location() -> *mut c_int;
}

fn errno() -> c_int {
    // SAFETY: musl's `__errno_location` returns a valid pointer for this thread.
    unsafe { *__errno_location() }
}

/// **Which of the paths Mesa might want can this process actually reach?**
///
/// `EGL_NOT_INITIALIZED` is EGL's way of saying *something went wrong* and
/// carries nothing else; `eglGetError` has one code for a dozen causes. Under
/// a Landlock ruleset the likeliest cause is a refused path, and the cheapest
/// way to turn that guess into data is to try them here and say which.
///
/// **This is a probe's job and not a library's.** It exists because the
/// alternative was another build-and-flash cycle per hypothesis, and there
/// were four.
fn reachable() -> String {
    let mut out = Vec::new();
    for path in [
        "/dev/dri",
        "/dev/dri/card0",
        "/dev/dri/card1",
        "/dev/dri/renderD128",
        "/sys/class/drm",
        "/sys/dev/char",
        "/sys/devices",
        "/usr/lib/gbm",
    ] {
        let meta = std::fs::metadata(path);
        let verdict = match meta {
            Ok(m) if m.is_dir() => match std::fs::read_dir(path) {
                Ok(_) => "listable",
                Err(e) => match e.kind() {
                    std::io::ErrorKind::PermissionDenied => "DENIED(list)",
                    _ => "unreadable",
                },
            },
            Ok(_) => match std::fs::OpenOptions::new().read(true).open(path) {
                Ok(_) => "openable",
                Err(e) => match e.kind() {
                    std::io::ErrorKind::PermissionDenied => "DENIED(open)",
                    _ => "unopenable",
                },
            },
            Err(e) => match e.kind() {
                std::io::ErrorKind::PermissionDenied => "DENIED(stat)",
                std::io::ErrorKind::NotFound => "absent",
                _ => "unstattable",
            },
        };
        out.push(format!("{path}={verdict}"));
    }
    out.join(" ")
}

fn main() {
    match run() {
        Ok(report) => {
            println!("{report}");
            // **Idle rather than exit**, `mesa-demo`'s ending for its reason:
            // a supervisor restarting a finished probe forever is worse than
            // one clear line, and leaving the picture up is what lets a person
            // look at the screen the measurement is about.
            println!("gpu-flip: holding the last frame; nothing further is drawn");
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        }
        Err(why) => {
            eprintln!("gpu-flip: {why}");
            std::process::exit(1);
        }
    }
}

fn run() -> Result<String, String> {
    // **The lease, over the socket the supervisor named.** Not a card node —
    // see the note at the top of this file for why there is no such thing for
    // an application. `next` blocks until the arbiter hands one over, which is
    // the ordinary start-up of every kiosk application on this platform.
    let channel = rivet_sdk::display::Channel::from_env().map_err(|e| {
        format!(
            "no display channel: {e}. This probe is a KIOSK application and must be started by \
             a supervisor that leases it the screen — it cannot open a card of its own, and \
             `rivet_core::service::device_globs` is where that is decided"
        )
    })?;
    let lease = channel
        .next()
        .map_err(|e| format!("the arbiter handed no lease: {e}"))?;
    let fd = lease.as_raw_fd();

    // **No `drmSetMaster`.** A lease already carries the right to flip the
    // objects in it; asking for master on a leased fd is asking for the whole
    // card, which is the arbiter's and is refused.
    let master = "not asked for — a lease already permits flipping its own objects";

    let (crtc_id, connector_id, mut mode, width, height) = pick_display(fd)?;

    // ---- GBM ---------------------------------------------------------------
    //
    // SAFETY: every call is a C function taking what its header declares. The
    // card descriptor outlives the device because `card` lives to the end of
    // `run` and nothing here closes it.
    // **Which descriptor will GBM accept?** — the diagnostic that separates two
    // very different failures, added 2026-09-20 after the confinement grants
    // landed and `MESA-LOADER: failed to retrieve device information` did not
    // move.
    //
    // A lessee is handed two things that are both DRM descriptors: the LEASE,
    // which carries the right to flip the leased connector and CRTC, and the
    // RENDER NODE, which `device_globs` grants it outright. If GBM refuses the
    // lease and accepts the render node, this is not a policy problem at all —
    // it is Mesa declining to treat a leased fd as a device, and the design
    // has to become the standard split: render on the render node, import the
    // buffer into the display fd as a dma-buf. That is a different and larger
    // piece of work, and it is worth one `open` to find out rather than
    // several build-and-flash cycles to guess.
    let render_node = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/dri/renderD128");

    // **GBM goes on the RENDER node, never on the lease.** Proven on the board
    // 2026-09-20: `gbm_create_device` accepts `/dev/dri/renderD128` and refuses
    // the lease, because the lease is on the DISPLAY device and that device
    // does not render. See the header.
    let render = render_node.map_err(|e| {
        format!("/dev/dri/renderD128: {e} — a lessee is granted the render node by \
                 `device_globs`, so this is not a policy refusal")
    })?;
    let render_fd = render.as_raw_fd();

    // **The LEASE is tried first, and the earlier conclusion that it could not
    // work was drawn too early.**
    //
    // When `gbm_create_device` was first refused on the lease, `/sys/class/drm`
    // and listing `/dev/dri` were still refused by the policy — so the lease
    // may have failed for the enumeration reason the render node later failed
    // for, rather than for being a lease. The render node happening to succeed
    // at that moment is not evidence that a lease cannot.
    //
    // It matters because of what the Pi 4 actually wants: Mesa's **renderonly**
    // layer knows this SoC splits render from scanout and handles it inside a
    // GBM device created on the DISPLAY device, allocating on V3D and sharing
    // with VC4 without anybody hand-rolling a PRIME import. If that works, it
    // is both simpler and the reason `drmPrimeFDToHandle` refuses a buffer this
    // program tried to bridge itself.
    //
    // So: ask, take the display device if it answers, and say which was used.
    // SAFETY: `fd` is the lease descriptor, live for this call.
    let on_lease = unsafe { gbm_create_device(fd) };
    let (gbm_fd, gbm_on) = if on_lease.is_null() {
        (render_fd, "the RENDER NODE (the lease was refused even with every grant in place)")
    } else {
        (fd, "the LEASE, via Mesa's renderonly layer — no hand-rolled PRIME needed")
    };

    let (gbm, surface, dpy, egl_surface) = unsafe {
        let gbm = if on_lease.is_null() { gbm_create_device(gbm_fd) } else { on_lease };
        if gbm.is_null() {
            // **Read the MESA-LOADER line above this one in the journal**, and
            // not this sentence: `gbm_create_device` returns NULL for several
            // unrelated reasons and Mesa has already said which. Both of the
            // ones seen on a board so far were confinement rather than
            // anything missing, and they read very differently:
            //
            //   "dri_gbm.so: Permission denied"          -> GBM_BACKEND_DIR
            //   "failed to retrieve device information"  -> DRM_SYSFS_*_DIR
            //
            // The first draft of this message asserted the backend was absent,
            // which was wrong twice over: the file was staged, and the cause
            // was a policy. A probe that names one cause for a call with
            // several is a probe that sends the next reader to the wrong file.
            return Err(format!(
                "gbm_create_device returned NULL on the LEASE — read the MESA-LOADER line above \
                 this one, which says why. `Permission denied` on a .so means the confinement \
                 policy (rivet_core::confinement) refused a path that IS on the card; `failed \
                 to retrieve device information` means it refused the sysfs Mesa identifies a \
                 device through. Neither is a missing file"
            ));
        }

        // **SCANOUT | RENDERING**, and the middle draft that dropped SCANOUT
        // was wrong for the same reason the PRIME import was: it assumed GBM
        // lived on the render node. It lives on the lease, so this allocation
        // must be scanout-capable or `drmModeAddFB2` refuses it — which it
        // did, with `EINVAL`, and the message blamed tiling.
        //
        // **LINEAR is NOT asked for**, and that is the corrected part.
        // `renderonly` knows what layout VC4 can scan out and V3D can render
        // into; a program that pins LINEAR here is overriding the one piece of
        // code on the machine that actually knows the answer.
        let surface = gbm_surface_create(
            gbm,
            width,
            height,
            GBM_FORMAT_XRGB8888,
            GBM_BO_USE_SCANOUT | GBM_BO_USE_RENDERING,
        );
        if surface.is_null() {
            return Err(format!(
                "gbm_surface_create refused {width}x{height} XRGB8888 with SCANOUT|RENDERING"
            ));
        }
        let _ = GBM_BO_USE_LINEAR;

        let dpy = eglGetPlatformDisplay(EGL_PLATFORM_GBM_KHR, gbm, std::ptr::null());
        if dpy.is_null() {
            return Err(format!("eglGetPlatformDisplay: EGL error {:#x}", eglGetError()));
        }
        let (mut major, mut minor) = (0, 0);
        if eglInitialize(dpy, &raw mut major, &raw mut minor) == 0 {
            return Err(format!(
                "eglInitialize: EGL error {:#x} (0x3001 is EGL_NOT_INITIALIZED, which carries \
                 no cause). REACHABILITY: {}",
                eglGetError(),
                reachable()
            ));
        }
        if eglBindAPI(EGL_OPENGL_ES_API) == 0 {
            return Err("eglBindAPI(OPENGL_ES) refused".to_string());
        }

        // **`EGL_WINDOW_BIT` here, unlike `mesa-demo`** — see that constant.
        let attrib = [
            EGL_SURFACE_TYPE,
            EGL_WINDOW_BIT,
            EGL_RENDERABLE_TYPE,
            EGL_OPENGL_ES2_BIT,
            EGL_RED_SIZE,
            8,
            EGL_GREEN_SIZE,
            8,
            EGL_BLUE_SIZE,
            8,
            EGL_ALPHA_SIZE,
            0,
            EGL_NONE,
        ];
        // **Several configs, then matched by native visual.** One config is
        // what `mesa-demo` took, and it is wrong for a window surface: EGL may
        // return a config whose native visual is a different fourcc than the
        // GBM surface was created with, and `eglCreateWindowSurface` then fails
        // with a mismatch that reads like a driver fault. Asking for a handful
        // and picking the one whose `EGL_NATIVE_VISUAL_ID` IS the format is the
        // standard remedy and costs one loop.
        let mut configs: [EGLConfig; 64] = [std::ptr::null_mut(); 64];
        let mut found: EGLint = 0;
        if eglChooseConfig(
            dpy,
            attrib.as_ptr(),
            configs.as_mut_ptr(),
            configs.len() as EGLint,
            &raw mut found,
        ) == 0
            || found == 0
        {
            return Err("eglChooseConfig found no ES2 window configuration".to_string());
        }
        let mut chosen: EGLConfig = std::ptr::null_mut();
        let mut advertised = Vec::new();
        for config in configs.iter().take(found as usize) {
            let mut visual: EGLint = 0;
            if eglGetConfigAttrib(dpy, *config, EGL_NATIVE_VISUAL_ID, &raw mut visual) != 0 {
                // **What the driver actually says**, collected because
                // `EGL_BAD_MATCH` on the window surface proved the fallback to
                // config zero was wrong and guessing a third time is not a
                // method. A fourcc is four ASCII bytes when it is one at all.
                let bytes = (visual as u32).to_le_bytes();
                let printable = bytes.iter().all(|b| b.is_ascii_graphic());
                advertised.push(if printable {
                    format!("{}", String::from_utf8_lossy(&bytes))
                } else {
                    format!("{visual:#x}")
                });
                if visual as u32 == GBM_FORMAT_XRGB8888 {
                    chosen = *config;
                }
            } else {
                advertised.push("unqueryable".to_string());
            }
        }
        if chosen.is_null() {
            return Err(format!(
                "no ES2 window config matches XR24. The {found} configs advertise \
                 EGL_NATIVE_VISUAL_ID: {}. If none is a fourcc, a GBM WINDOW SURFACE is the \
                 wrong abstraction on a render node and this must render into a gbm_bo through \
                 an EGLImage and an FBO instead — which is the split-SoC path and what \
                 mesa-demo already does half of",
                advertised.join(" ")
            ));
        }
        // **No match is the ORDINARY case on a render node**, and treating it
        // as a failure was wrong. `EGL_NATIVE_VISUAL_ID` describes a visual of
        // the *display* the configs belong to — and these configs belong to
        // V3D, which has no display and therefore no visuals. The board
        // answered with all sixteen ES2 window configurations and not one
        // carrying XR24.
        //
        // The match is still attempted first, because on a machine where
        // render and scanout ARE the same device it is the right answer and
        // picking blindly there is how a surface ends up in a format the CRTC
        // will not take. `kmscube` falls back the same way and for the same
        // reason.
        //
        // What makes the fallback safe here is that the format is enforced
        // somewhere better anyway: `gbm_surface_create` was given
        // `GBM_FORMAT_XRGB8888` outright, so the buffers this surface hands
        // back are XR24 whatever EGL says about visuals.
        if chosen.is_null() {
            chosen = configs[0];
        }

        let ctx_attrib = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
        let ctx = eglCreateContext(dpy, chosen, std::ptr::null_mut(), ctx_attrib.as_ptr());
        if ctx.is_null() {
            return Err(format!("eglCreateContext: EGL error {:#x}", eglGetError()));
        }
        let egl_surface = eglCreateWindowSurface(dpy, chosen, surface, std::ptr::null());
        if egl_surface.is_null() {
            return Err(format!(
                "eglCreateWindowSurface over the GBM surface: EGL error {:#x}",
                eglGetError()
            ));
        }
        if eglMakeCurrent(dpy, egl_surface, egl_surface, ctx) == 0 {
            return Err(format!("eglMakeCurrent: EGL error {:#x}", eglGetError()));
        }
        (gbm, surface, dpy, egl_surface)
    };

    // **The line that distinguishes the hardware from the software rasteriser**
    // — `mesa-demo`'s whole argument, repeated because this program could
    // equally well be flipping llvmpipe's output and every other line would
    // look the same.
    // SAFETY: a current context makes `glGetString` valid; the pointer is a
    // static string owned by the driver.
    let renderer = unsafe {
        let p = glGetString(GL_RENDERER);
        if p.is_null() {
            "unknown".to_string()
        } else {
            std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    };

    // ---- The flip loop ------------------------------------------------------
    let mut previous: *mut c_void = std::ptr::null_mut();
    let mut previous_fb: u32 = 0;
    let mut first = true;
    let began = std::time::Instant::now();
    let mut worst = std::time::Duration::ZERO;

    for frame in 0..FRAMES {
        #[allow(clippy::cast_precision_loss)]
        let t = frame as f32 / FRAMES as f32;
        // SAFETY: a current context; these are plain state calls.
        unsafe {
            glClearColor(t, 1.0 - t, 0.25, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            glFinish();
            if eglSwapBuffers(dpy, egl_surface) == 0 {
                return Err(format!("eglSwapBuffers: EGL error {:#x}", eglGetError()));
            }
        }

        // SAFETY: the surface is the one just swapped, so it has a front buffer.
        let bo = unsafe { gbm_surface_lock_front_buffer(surface) };
        if bo.is_null() {
            return Err(format!(
                "gbm_surface_lock_front_buffer returned NULL at frame {frame} — the surface has \
                 no free buffer, which means a previous one was never released"
            ));
        }
        // **The crossing.** `bo` belongs to V3D and the CRTC belongs to VC4,
        // so the GEM handle `gbm_bo_get_handle` would give is meaningless on
        // the lease. A dma-buf descriptor is the currency the two devices
        // share: export it from one, import it into the other, and the second
        // device issues a handle of its own for the same pages.
        //
        // **The handle is already this device's, and the PRIME dance was the
        // mistake.** GBM is on the lease, so Mesa's `renderonly` layer has
        // already allocated on V3D and shared the result with VC4; the buffer
        // `gbm_surface_lock_front_buffer` hands back is a VC4 buffer. Asking
        // `drmPrimeFDToHandle` to import it a second time is what answered
        // `EINVAL` — it was being told to import a buffer the device already
        // owns.
        //
        // This file previously carried a note on `gbm_bo_get_handle` calling
        // it the thing NOT to reach for, on the reasoning that a handle from
        // one DRM device is meaningless on another. The reasoning is sound and
        // the premise was wrong: there is only one device involved here,
        // because renderonly hid the second one.
        //
        // SAFETY: `bo` is a live buffer object from this surface.
        let (handle, stride) =
            unsafe { (gbm_bo_get_handle(bo) as u32, gbm_bo_get_stride(bo)) };
        let _ = (drmPrimeFDToHandle, gbm_bo_get_fd, close, gbm_on);

        let mut fb_id: u32 = 0;
        let handles = [handle, 0, 0, 0];
        let pitches = [stride, 0, 0, 0];
        let offsets = [0u32; 4];
        // SAFETY: the four arrays are the length `drmModeAddFB2` reads for a
        // single-plane format, and `fb_id` is written by the kernel.
        let rc = unsafe {
            drmModeAddFB2(
                fd,
                width,
                height,
                DRM_FORMAT_XRGB8888,
                handles.as_ptr(),
                pitches.as_ptr(),
                offsets.as_ptr(),
                &raw mut fb_id,
                0,
            )
        };
        if rc != 0 {
            return Err(format!(
                "drmModeAddFB2 refused the imported buffer at frame {frame}: {rc}, errno {} — \
                 the import succeeded and the display controller will not take it as a \
                 framebuffer. A non-linear layout is the usual cause",
                errno()
            ));
        }
        let _ = drmModeAddFB;

        let step = std::time::Instant::now();
        // `drmModeSetCrtc` takes a mutable pointer to an ARRAY of connector
        // ids, so the id needs a binding to point at rather than a temporary.
        let mut connectors = [connector_id];
        if first {
            // SAFETY: mode and connector came from this card's resources.
            let rc = unsafe {
                drmModeSetCrtc(
                    fd,
                    crtc_id,
                    fb_id,
                    0,
                    0,
                    connectors.as_mut_ptr(),
                    1,
                    &raw mut mode,
                )
            };
            if rc != 0 {
                return Err(format!("drmModeSetCrtc: {rc}, errno {}", errno()));
            }
            first = false;
        } else {
            // SAFETY: as `rivet_sdk::screen::Screen::flip`.
            let rc = unsafe {
                drmModePageFlip(fd, crtc_id, fb_id, PAGE_FLIP_EVENT, std::ptr::null_mut())
            };
            if rc != 0 {
                return Err(format!(
                    "drmModePageFlip to a GBM framebuffer refused at frame {frame}: {rc}, \
                     errno {}",
                    errno()
                ));
            }
            await_flip(fd)?;
        }
        worst = worst.max(step.elapsed());

        // Release the buffer the PREVIOUS frame was scanning out, never the one
        // that is now on the glass.
        if !previous.is_null() {
            // SAFETY: `previous` was locked from this surface and is no longer
            // being scanned out, the flip above having completed.
            unsafe { gbm_surface_release_buffer(surface, previous) };
            // SAFETY: `previous_fb` was created by this process.
            unsafe { drmModeRmFB(fd, previous_fb) };
        }
        previous = bo;
        previous_fb = fb_id;
    }

    let elapsed = began.elapsed();
    #[allow(clippy::cast_precision_loss)]
    let per = elapsed.as_secs_f64() * 1000.0 / f64::from(FRAMES);
    let _ = gbm;
    // The lease must outlive every object made on it — the GBM device, the EGL
    // display and the framebuffers all reference this descriptor.
    std::mem::forget(lease);

    Ok(format!(
        "gpu-flip: GL_RENDERER = {renderer}\n\
         gpu-flip: {width}x{height} on crtc {crtc_id}, connector {connector_id}, \
         master {master}\n\
         gpu-flip: {FRAMES} GBM buffers scanned out — mean {per:.2} ms/frame \
         ({:.1} fps), worst flip {:.2} ms\n\
         gpu-flip: THE GPU'S OWN BUFFER REACHED THE GLASS. A `gbm_bo` became a \
         DRM framebuffer and the CRTC flipped to it, so nothing is copied back \
         across the bus.",
        1000.0 / per,
        worst.as_secs_f64() * 1000.0,
    ))
}

/// The first connected connector, its preferred mode, and a CRTC for it.
///
/// `rivet_sdk::screen`'s `drive`, narrowed: this takes the encoder's current
/// CRTC and does not search `possible_crtcs`, because a probe that cannot find
/// a display on a board with one screen attached has found something worth
/// reporting rather than something worth working around.
fn pick_display(fd: c_int) -> Result<(u32, u32, ModeInfo, u32, u32), String> {
    // SAFETY: `fd` is an open DRM card node.
    let res = unsafe { drmModeGetResources(fd) };
    if res.is_null() {
        return Err(format!("drmModeGetResources: errno {}", errno()));
    }
    // SAFETY: non-null, from libdrm, and freed on every path below.
    let r = unsafe { &*res };
    let connectors =
        // SAFETY: libdrm guarantees `count_connectors` entries at `connectors`.
        unsafe { std::slice::from_raw_parts(r.connectors, r.count_connectors.max(0) as usize) };

    for &id in connectors {
        // SAFETY: `id` came from this card's resource list.
        let conn = unsafe { drmModeGetConnector(fd, id) };
        if conn.is_null() {
            continue;
        }
        // SAFETY: non-null and freed before this iteration ends.
        let c = unsafe { &*conn };
        if c.connection != CONNECTED || c.count_modes <= 0 || c.encoder_id == 0 {
            // SAFETY: from `drmModeGetConnector`.
            unsafe { drmModeFreeConnector(conn) };
            continue;
        }
        // SAFETY: `count_modes` entries at `modes`; mode 0 is the preferred one.
        let mode = unsafe { std::ptr::read(c.modes) };
        let (w, h) = (u32::from(mode.hdisplay), u32::from(mode.vdisplay));
        // SAFETY: `encoder_id` is non-zero and from this card.
        let enc = unsafe { drmModeGetEncoder(fd, c.encoder_id) };
        if enc.is_null() {
            // SAFETY: from `drmModeGetConnector`.
            unsafe { drmModeFreeConnector(conn) };
            continue;
        }
        // SAFETY: non-null, freed immediately below.
        let crtc_id = unsafe { (*enc).crtc_id };
        // SAFETY: both came from libdrm.
        unsafe {
            drmModeFreeEncoder(enc);
            drmModeFreeConnector(conn);
        }
        if crtc_id == 0 {
            continue;
        }
        // SAFETY: from `drmModeGetResources`.
        unsafe { drmModeFreeResources(res) };
        return Ok((crtc_id, id, mode, w, h));
    }

    // SAFETY: from `drmModeGetResources`.
    unsafe { drmModeFreeResources(res) };
    Err("no connected connector with a mode and a CRTC — is a screen plugged in?".to_string())
}

/// Wait for `DRM_EVENT_FLIP_COMPLETE`, or give up.
///
/// `rivet_sdk::screen::Screen::await_flip`, reduced to its essentials: a
/// `poll` with an absolute deadline and a read that drains whatever arrived.
/// The event's contents are not inspected, because only one flip is ever in
/// flight.
fn await_flip(fd: c_int) -> Result<(), String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    let mut buf = [0u8; 1024];
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return Err("no DRM_EVENT_FLIP_COMPLETE in 500 ms".to_string());
        }
        let mut pollfd = PollFd { fd, events: POLLIN, revents: 0 };
        // SAFETY: one descriptor, valid for this call.
        let rc = unsafe {
            poll(&raw mut pollfd, 1, c_int::try_from(left.as_millis()).unwrap_or(c_int::MAX).max(1))
        };
        if rc < 0 {
            if errno() == EINTR {
                continue;
            }
            return Err(format!("poll on the card: errno {}", errno()));
        }
        if rc == 0 {
            continue;
        }
        // SAFETY: the card is readable and `buf` is this frame's.
        let n = unsafe { read(fd, buf.as_mut_ptr().cast::<c_void>(), buf.len()) };
        if n > 0 {
            return Ok(());
        }
    }
}

const POLLIN: i16 = 0x001;
const EINTR: c_int = 4;

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: i16,
    revents: i16,
}

unsafe extern "C" {
    fn poll(fds: *mut PollFd, nfds: c_uint, timeout: c_int) -> c_int;
    fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
}

/// Unused today; kept so the linker proves `CString` is reachable the way
/// `mesa-demo` does for its shader sources.
#[allow(dead_code)]
fn unused() -> CString {
    CString::new("gpu-flip").unwrap_or_default()
}
