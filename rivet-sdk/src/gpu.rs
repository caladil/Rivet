//! **A screen an application draws on with the GPU** — `.rulings/` R-206,
//! answering `ADR-058` §5's open question with *a dma-buf*.
//!
//! [`crate::composited`] is the CPU path: an application writes pixels into a
//! `&[u8]`, the SDK copies them into shared memory, and the compositor uploads
//! them as a texture every frame. This is the same surface with the copies
//! taken out. The application renders with GL into a buffer on its own render
//! node, and what crosses to the compositor is a **descriptor**, not pixels.
//!
//! # Why this exists, measured
//!
//! `GPU-Backgrounds-2026-09-20` §1, on `rivet-01`:
//!
//! ```text
//! cost over 9000 frames drawing Contours:
//!   background 32.12 ms, DRAWING 41.77 ms of the 33.3 ms budget (125%)
//! ```
//!
//! Aurora and Contours are a 96x54 scalar field sampled per pixel and scaled to
//! two megapixels. On the V3D that is one textured quad with the texture unit
//! doing the filtering in hardware, and the ARM cores are spending 32 ms a
//! frame on it. A background that misses the budget is a defect by `T8`'s own
//! standard, so those two do not ship as CPU code.
//!
//! # The trap this shape exists to avoid
//!
//! §3 of that document: **a half-GPU renderer is slower than an all-CPU one.**
//! If the frame is drawn on the GPU and then read back so the CPU can composite
//! it, the readback of 8.29 MB at 1080p costs more than the 32 ms being saved.
//! The win exists only if the frame **stays** on the GPU from the first
//! fragment to the glass — which is exactly what a dma-buf buys and what
//! `wl_shm` cannot do at any price.
//!
//! # What an application declares
//!
//! ```toml
//! devices  = ["gpu"]                          # R-206 D4: a render node, not a screen
//! connects = ["rivet-displayd:wayland"]       # the surface, as ever
//! ```
//!
//! `gpu` grants `/dev/dri/renderD*` and nothing else, in either display role,
//! gated by a `const` assertion in `rivet-core`. **This application still has
//! no card node and still cannot modeset** — it renders into its own buffer and
//! hands the compositor a handle. `ADR-058`'s *"an application does not get the
//! screen"* is untouched.
//!
//! # It is NOT a static binary, and that is the honest cost
//!
//! [`crate::composited`] adds no C library — `smithay-client-toolkit` with
//! `default-features = false` is pure Rust and `cmd/rivet-launcher` is still
//! static. This module links `libgbm`, `libEGL` and `libGLESv2`, which ship
//! shared-only, so `ADR-003` §5 takes the same exception here that
//! `examples/probes/gpu-flip` and `cmd/rivet-displayd` already take. All three
//! are `PUBLIC_LIBRARIES`, so an application's `library = []` stays true.
//!
//! **The bindings are direct FFI and not a crate**, which is `mesa-demo`'s rule
//! and `gpu-flip`'s: a failure in `gbm-rs` and a failure in `gbm` look
//! identical from a board, and there is no way to tell them apart at three in
//! the morning. Every declaration below is copied from `gpu-flip`, which ran on
//! `rivet-06f5c`.

// **`unsafe` is the point of this module**, exactly as it is in `screen.rs`:
// GBM, EGL and GL are C libraries and there is no safe spelling of an EGL
// context. Every declaration below is copied from `examples/probes/gpu-flip`,
// which ran on `rivet-06f5c` — read out of a working program rather than
// recalled, because a signature one argument out does not fail to compile, it
// reads the wrong register and produces a black screen.
//
// **What is NOT unsafe here is the lifetime discipline**, and it is the part
// that bites, twice over. Each buffer object has one `wl_buffer` for exactly as
// long as GBM keeps the object, and no longer — `crate::attached` says why the
// object is not kept for the surface's life. And each object must be given back
// to GBM when the compositor releases it: `Screen::reclaim` is the whole of
// that rule, and `gpu-flip`'s own note records what happens without it —
// `lock_front_buffer` starts returning NULL within a few frames.
#![allow(unsafe_code)]

use std::ffi::{c_char, c_int, c_void};

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::client::globals::{registry_queue_init, GlobalList};
use smithay_client_toolkit::reexports::client::protocol::{
    wl_buffer, wl_output, wl_surface,
};
use smithay_client_toolkit::reexports::client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::shell::xdg::window::{
    Window, WindowConfigure, WindowDecorations, WindowHandler,
};
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::subcompositor::SubcompositorState;
use smithay_client_toolkit::{
    delegate_compositor, delegate_output, delegate_registry, delegate_shm, delegate_subcompositor,
    delegate_xdg_shell, delegate_xdg_window, registry_handlers,
};

use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1::{self, ZwpLinuxBufferParamsV1},
    zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
};

use crate::attached::{self, Destroy, UserData};

/// Where the compositor accepts connections — the same socket
/// [`crate::composited`] uses, because it is the same compositor and the same
/// endpoint. Re-declared rather than imported for that module's reason.
pub use crate::composited::SOCKET;

/// How long the compositor may be silent before it is asked whether it is
/// still there. [`crate::composited`]'s constant
/// and its argument; duplicated rather than shared because the two modules are
/// alternatives and a change to one is not automatically right for the other.
const CONFIGURE_PATIENCE: std::time::Duration = std::time::Duration::from_secs(10);

/// Where the render nodes are. `Device::Gpu` grants `renderD*` and this is the
/// directory it grants them in.
const DRM_NODE_DIR: &str = "/dev/dri";

// ---- GBM ------------------------------------------------------------------

/// `GBM_FORMAT_XRGB8888` — `'X' 'R' '2' '4'` little-endian.
///
/// **The same fourcc `crate::composited` hands `wl_shm`**, and deliberately so:
/// an application that moves between the two paths does not change what it
/// believes a pixel looks like.
const GBM_FORMAT_XRGB8888: u32 = 0x3432_5258;

const GBM_BO_USE_RENDERING: u32 = 1 << 2;
/// **`GBM_BO_USE_LINEAR`, and on this board it is not optional** — `gpu-flip`
/// found this the hard way and the note is worth carrying verbatim.
///
/// The buffer is allocated by **V3D** and consumed by the display side, which
/// on a Pi 4 under `vc4-kms-v3d` is a different DRM device. V3D's preferred
/// tiling means nothing to VC4's display FIFO, and a tiled buffer imported
/// across the two **shows as a scrambled screen rather than as an error**.
/// Linear is the layout both agree on, and it is what every split-SoC
/// compositor asks for at this seam.
const GBM_BO_USE_LINEAR: u32 = 1 << 4;

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
    fn gbm_surface_lock_front_buffer(surface: *mut c_void) -> *mut c_void;
    /// Give it back once the compositor has released the `wl_buffer`, or the
    /// surface runs out of buffers and `lock_front_buffer` starts returning
    /// NULL — the leak `gpu-flip` hit within a few frames.
    fn gbm_surface_release_buffer(surface: *mut c_void, bo: *mut c_void);
    fn gbm_bo_get_stride(bo: *mut c_void) -> u32;
    /// **Export the buffer as a dma-buf.** The one call this whole module is
    /// arranged around: a descriptor is the only currency two DRM devices
    /// share, because a GEM handle is meaningless outside the device that
    /// issued it.
    fn gbm_bo_get_fd(bo: *mut c_void) -> c_int;
    fn gbm_bo_get_modifier(bo: *mut c_void) -> u64;
    fn gbm_bo_get_offset(bo: *mut c_void, plane: c_int) -> u32;
    /// **The object's one slot for its owner's data**, and the call GBM makes
    /// with it from inside `gbm_bo_destroy`. It is how this module learns that
    /// Mesa has destroyed an object, which Mesa does on its own schedule.
    fn gbm_bo_set_user_data(bo: *mut c_void, data: *mut c_void, destroy: Option<Destroy>);
    fn gbm_bo_get_user_data(bo: *mut c_void) -> *mut c_void;
}

/// GBM's user-data slot, as [`attached`] asks for it.
struct GbmUserData;

impl UserData for GbmUserData {
    unsafe fn get(&self, bo: *mut c_void) -> *mut c_void {
        // SAFETY: the trait's contract — `bo` is a live buffer object.
        unsafe { gbm_bo_get_user_data(bo) }
    }

    unsafe fn set(&self, bo: *mut c_void, data: *mut c_void, destroy: Destroy) {
        // SAFETY: the trait's contract — `bo` is a live buffer object.
        unsafe { gbm_bo_set_user_data(bo, data, Some(destroy)) };
    }
}

/// **The `wl_buffer` a buffer object owns**, destroyed when the object is.
///
/// Destroying it is what tells the compositor to let go of the object's
/// dma-buf. GBM drops this from inside `gbm_bo_destroy`, which Mesa calls from
/// inside EGL on this thread, and only for an object the compositor has
/// finished with: Mesa destroys an object only while it is unlocked, and
/// [`Screen::reclaim`] unlocks one only after the compositor released its
/// `wl_buffer`. Sending the destroy request cannot panic: on a connection that
/// has gone, it is dropped.
struct Owned(wl_buffer::WlBuffer);

impl Drop for Owned {
    fn drop(&mut self) {
        self.0.destroy();
    }
}

// ---- EGL, copied from `gpu-flip`, which copied `mesa-demo` ----------------

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
const EGL_NATIVE_VISUAL_ID: EGLint = 0x302E;
/// `EGL_WINDOW_BIT` — a GBM surface is a window, and `mesa-demo` records the
/// mirror-image mistake of asking for `EGL_PBUFFER_BIT` against a GBM display.
const EGL_WINDOW_BIT: EGLint = 0x0004;

const GL_RENDERER: u32 = 0x1F01;

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

// **The GLES constants and entry points [`Screen::show_pixels`] needs**, and
// nothing beyond them. An application drawing with the GPU links `libGLESv2`
// itself and calls whatever it likes; this crate names only what it uses.
const GL_FRAGMENT_SHADER: u32 = 0x8B30;
const GL_VERTEX_SHADER: u32 = 0x8B31;
const GL_COMPILE_STATUS: u32 = 0x8B81;
const GL_LINK_STATUS: u32 = 0x8B82;
const GL_ARRAY_BUFFER: u32 = 0x8892;
const GL_STATIC_DRAW: u32 = 0x88E4;
const GL_FLOAT: u32 = 0x1406;
const GL_TRIANGLE_STRIP: u32 = 0x0005;
const GL_TEXTURE_2D: u32 = 0x0DE1;
const GL_TEXTURE0: u32 = 0x84C0;
const GL_RGBA: u32 = 0x1908;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_TEXTURE_MIN_FILTER: u32 = 0x2801;
const GL_TEXTURE_MAG_FILTER: u32 = 0x2800;
const GL_TEXTURE_WRAP_S: u32 = 0x2802;
const GL_TEXTURE_WRAP_T: u32 = 0x2803;
const GL_NEAREST: i32 = 0x2600;
const GL_CLAMP_TO_EDGE: i32 = 0x812F;
const GL_UNPACK_ALIGNMENT: u32 = 0x0CF5;

#[link(name = "GLESv2")]
unsafe extern "C" {
    fn glGetString(name: u32) -> *const c_char;
    fn glCreateShader(kind: u32) -> u32;
    fn glShaderSource(shader: u32, count: i32, string: *const *const c_char, length: *const i32);
    fn glCompileShader(shader: u32);
    fn glGetShaderiv(shader: u32, pname: u32, params: *mut i32);
    fn glCreateProgram() -> u32;
    fn glAttachShader(program: u32, shader: u32);
    fn glLinkProgram(program: u32);
    fn glGetProgramiv(program: u32, pname: u32, params: *mut i32);
    fn glUseProgram(program: u32);
    fn glGetAttribLocation(program: u32, name: *const c_char) -> i32;
    fn glGetUniformLocation(program: u32, name: *const c_char) -> i32;
    fn glUniform1i(location: i32, v: i32);
    fn glGenBuffers(n: i32, buffers: *mut u32);
    fn glBindBuffer(target: u32, buffer: u32);
    fn glBufferData(target: u32, size: isize, data: *const c_void, usage: u32);
    fn glEnableVertexAttribArray(index: u32);
    fn glVertexAttribPointer(
        index: u32,
        size: i32,
        kind: u32,
        normalized: u8,
        stride: i32,
        pointer: *const c_void,
    );
    fn glGenTextures(n: i32, textures: *mut u32);
    fn glBindTexture(target: u32, texture: u32);
    fn glActiveTexture(unit: u32);
    fn glTexImage2D(
        target: u32,
        level: i32,
        internal: i32,
        width: i32,
        height: i32,
        border: i32,
        format: u32,
        kind: u32,
        pixels: *const c_void,
    );
    fn glTexParameteri(target: u32, pname: u32, param: i32);
    fn glPixelStorei(pname: u32, param: i32);
    fn glDrawArrays(mode: u32, first: i32, count: i32);
    fn glViewport(x: i32, y: i32, width: i32, height: i32);
}

/// A full-screen triangle strip in clip space with texture coordinates.
///
/// **`v` runs from 1 at the bottom to 0 at the top**, which flips the image:
/// a CPU canvas's row 0 is the TOP of the picture and GL's texture origin is
/// the bottom. Without this the screen is upside down — which is obvious on a
/// clock and not at all obvious on a symmetric background.
const UPLOAD_QUAD: [f32; 16] = [
    -1.0, -1.0, 0.0, 1.0, //
    1.0, -1.0, 1.0, 1.0, //
    -1.0, 1.0, 0.0, 0.0, //
    1.0, 1.0, 1.0, 0.0,
];

const UPLOAD_VERTEX_SRC: &str = "attribute vec2 position;\n\
                                 attribute vec2 uv_in;\n\
                                 varying vec2 uv;\n\
                                 void main() {\n\
                                 \x20 uv = uv_in;\n\
                                 \x20 gl_Position = vec4(position, 0.0, 1.0);\n\
                                 }\n";

/// **`.bgr`, and that swizzle is the whole of the format conversion.**
///
/// A Rivet canvas is `XRGB8888` little-endian, so its bytes are B, G, R, X.
/// Uploaded as `GL_RGBA` those land in the R, G, B and A channels in that
/// order — red and blue swapped. `EXT_texture_format_BGRA8888` would fix it
/// at upload and is an extension this crate would then have to require;
/// reading `.bgr` in the shader costs nothing and works everywhere.
///
/// Alpha is written as `1.0` and the sampled `.a` is discarded: the fourth
/// byte of an `XRGB` canvas is padding (`crate::text::Format`), and a
/// background is opaque.
const UPLOAD_FRAGMENT_SRC: &str = "precision mediump float;\n\
                                   varying vec2 uv;\n\
                                   uniform sampler2D frame;\n\
                                   void main() {\n\
                                   \x20 gl_FragColor = vec4(texture2D(frame, uv).bgr, 1.0);\n\
                                   }\n";

/// A screen an application renders into with GL.
///
/// **Deliberately NOT the same shape as [`crate::composited::Screen`]**, and
/// that difference is the point rather than an oversight. The CPU one takes a
/// finished `&[u8]`; there is no `&[u8]` here and inventing one would mean the
/// readback §3 rules out. What an application gets instead is a current GL
/// context and [`Screen::present`].
pub struct Screen {
    inner: Inner,
    queue: EventQueue<Inner>,
    handle: QueueHandle<Inner>,
    /// The connection, kept for the life of the surface. Nothing calls it:
    /// `present` reads through the queue's own `prepare_read`
    /// (`composited::pump`).
    _conn: Connection,

    /// The surface's width in pixels, as the compositor configured it.
    pub width: usize,
    /// The surface's height in pixels.
    pub height: usize,
    /// Which compositor, how big, which renderer — the line an application
    /// puts in its journal. **It names `GL_RENDERER`**, because *"drawing on
    /// the GPU"* and *"drawing on llvmpipe"* look identical from every other
    /// vantage point and differ by a factor of thirty.
    pub what: String,

    egl: Egl,
    /// **Held because dropping it would take the surface with it**, exactly as
    /// `cmd/rivet-displayd`'s `_gbm` is. Nothing reads it after construction
    /// and nothing should: the EGL display was created *from* it and every
    /// buffer below was allocated *on* it, so its lifetime is the floor under
    /// everything else in this struct.
    _gbm_device: *mut c_void,
    gbm_surface: *mut c_void,
    /// Buffers handed to the compositor and not yet released, newest last.
    /// Each goes back to GBM when the compositor releases it; its `wl_buffer`
    /// stays with the object, in GBM's user data, for the object's next turn.
    in_flight: Vec<InFlight>,
    /// **`None` until [`Screen::show_pixels`] is first called**, which is
    /// never on a host whose backgrounds all draw with the GPU.
    uploader: Option<Uploader>,
}

/// The one program and one texture [`Screen::show_pixels`] needs.
struct Uploader {
    program: u32,
    texture: u32,
    quad: u32,
    u_frame: i32,
    /// **Re-established every frame, not once.** An addon drawing with the
    /// GPU may bind its own buffer and repoint these — `rivetos_addon::gl`
    /// says in as many words that a pack need not restore GL state, because
    /// a host cannot trust a third-party object to unwind it correctly. So
    /// the host re-binds what it depends on, which is what these are for.
    position: u32,
    uv_in: u32,
}

/// One buffer the compositor is holding.
struct InFlight {
    bo: *mut c_void,
    buffer: wl_buffer::WlBuffer,
}

struct Egl {
    display: EGLDisplay,
    context: EGLContext,
    surface: EGLSurface,
}

/// Ask the compositor for a surface and build a GL context that renders into
/// it.
///
/// # Errors
///
/// The render node, GBM, EGL, the compositor socket, a missing
/// `zwp_linux_dmabuf` global, or a configure that never arrived. Every one
/// names which, because the journal is the only witness a headless board has.
///
/// **A compositor with no `zwp_linux_dmabuf` is a NAMED refusal here**, not a
/// black screen: it means `rivet-displayd` is older than R-206, and an
/// application that fell back to `wl_shm` silently would be the half-GPU
/// renderer §3 rules out.
#[allow(clippy::too_many_lines)]
pub fn take() -> Result<Screen, String> {
    let (node, fd) = open_render_node()?;

    // SAFETY: `fd` is an open DRM render node this process owns for its
    // lifetime; `gbm_create_device` either takes it or returns NULL.
    let gbm_device = unsafe { gbm_create_device(fd) };
    if gbm_device.is_null() {
        return Err(format!(
            "{node}: gbm_create_device returned NULL. The usual cause is the GBM backend not \
             being reachable — it is dlopened from /usr/lib/gbm, which the supervisor grants \
             only to an application declaring `display` or `gpu`"
        ));
    }

    let egl_display = unsafe {
        eglGetPlatformDisplay(EGL_PLATFORM_GBM_KHR, gbm_device, std::ptr::null())
    };
    if egl_display.is_null() {
        return Err(format!(
            "{node}: eglGetPlatformDisplay(EGL_PLATFORM_GBM_KHR) returned no display (EGL error \
             {:#x})",
            unsafe { eglGetError() }
        ));
    }
    if unsafe { eglInitialize(egl_display, std::ptr::null_mut(), std::ptr::null_mut()) } == 0 {
        return Err(format!("{node}: eglInitialize failed (EGL error {:#x})", unsafe {
            eglGetError()
        }));
    }
    if unsafe { eglBindAPI(EGL_OPENGL_ES_API) } == 0 {
        return Err("eglBindAPI(EGL_OPENGL_ES_API) failed".to_string());
    }

    // **The config must match the buffer format**, and `EGL_NATIVE_VISUAL_ID`
    // is how EGL says which fourcc a config will produce. Asking for 8/8/8 and
    // then creating an XRGB8888 GBM surface against a config whose visual is
    // something else is the classic way to get a working context that draws
    // the wrong colours.
    let config_attrs = [
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
        EGL_NONE,
    ];
    let mut configs = [std::ptr::null_mut::<c_void>(); 32];
    let mut found: EGLint = 0;
    if unsafe {
        eglChooseConfig(
            egl_display,
            config_attrs.as_ptr(),
            configs.as_mut_ptr(),
            32,
            &raw mut found,
        )
    } == 0
        || found == 0
    {
        return Err(format!(
            "{node}: no EGL config offers an ES2 window at 8/8/8 (EGL error {:#x})",
            unsafe { eglGetError() }
        ));
    }

    let config = pick_config(egl_display, &configs, found).ok_or_else(|| {
        format!("{node}: {found} EGL config(s) and none has EGL_NATIVE_VISUAL_ID XRGB8888")
    })?;

    let ctx_attrs = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
    let context =
        unsafe { eglCreateContext(egl_display, config, std::ptr::null_mut(), ctx_attrs.as_ptr()) };
    if context.is_null() {
        return Err(format!("{node}: eglCreateContext failed (EGL error {:#x})", unsafe {
            eglGetError()
        }));
    }

    // ---- the Wayland side, which decides how big the GBM surface is -------

    let stream = std::os::unix::net::UnixStream::connect(SOCKET).map_err(|e| {
        format!(
            "{SOCKET}: {e} — this application may not have been granted the compositor. It is \
             granted by declaring connects = [\"rivet-displayd:wayland\"] in the manifest, and \
             rivet-displayd has to be enabled on this machine"
        )
    })?;

    // **The deadline starts here, before any Wayland traffic at all — B10.**
    // See `composited::take`'s identical comment and `bounded_registry_init`
    // below for why registry discovery needs its own watchdog rather than
    // just a bound on the configure wait that follows it.
    let deadline = std::time::Instant::now() + CONFIGURE_PATIENCE;
    let watchdog = stream.try_clone().map_err(|e| {
        format!("{SOCKET}: the compositor socket could not be duplicated for the start-up deadline: {e}")
    })?;

    let conn = Connection::from_socket(stream)
        .map_err(|e| format!("{SOCKET}: the compositor would not talk Wayland: {e}"))?;
    let (globals, mut queue) = bounded_registry_init(&conn, watchdog, deadline)?;
    let handle = queue.handle();

    let compositor = CompositorState::bind(&globals, &handle)
        .map_err(|e| format!("no wl_compositor: {e} — this is not a compositor"))?;
    let shell = XdgShell::bind(&globals, &handle)
        .map_err(|e| format!("no xdg_shell: {e} — there is no way to ask for a window"))?;
    let shm = Shm::bind(&globals, &handle)
        .map_err(|e| format!("no wl_shm: {e} — there is no way to hand over widget pixels"))?;
    let subcompositor = SubcompositorState::bind(compositor.wl_compositor().clone(), &globals, &handle)
        .map_err(|e| format!("no wl_subcompositor: {e} — widgets cannot be layered over the background"))?;
    let dmabuf = bind_dmabuf(&globals, &handle)?;

    let surface = compositor.create_surface(&handle);
    let window = shell.create_window(surface, WindowDecorations::RequestServer, &handle);
    window.set_app_id("org.rivet.application");
    window.commit();

    let mut inner = Inner {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &handle),
        window,
        dmabuf,
        width: 0,
        height: 0,
        closed: false,
        released: Vec::new(),
        releases_seen: 0,
        overlay: None,
        shm,
        subcompositor,
    };

    // **Wait for the first configure for as long as the compositor answers** —
    // `crate::composited::wait_for_configure`, which keeps B10's bound on a
    // compositor that stops answering and drops it for one that is waiting
    // for a display (`.rulings/` R-236). This is start-up only; `Screen::pump`
    // below is the steady-state dispatch and is deliberately untouched — see
    // its own doc comment.
    crate::composited::wait_for_configure(
        &conn,
        &mut queue,
        &mut inner,
        CONFIGURE_PATIENCE,
        |inner| inner.width != 0 && inner.height != 0,
        || {
            eprintln!(
                "rivet-sdk: the compositor has no display yet; this GPU surface is sized and \
                 drawn when one is connected"
            );
        },
    )
    .map_err(|e| format!("{SOCKET}: {e}"))?;
    let (width, height) = (inner.width, inner.height);

    // SAFETY: `gbm_device` is live and the dimensions came from a configure.
    let gbm_surface = unsafe {
        gbm_surface_create(
            gbm_device,
            u32::try_from(width).unwrap_or(0),
            u32::try_from(height).unwrap_or(0),
            GBM_FORMAT_XRGB8888,
            GBM_BO_USE_RENDERING | GBM_BO_USE_LINEAR,
        )
    };
    if gbm_surface.is_null() {
        return Err(format!(
            "{node}: gbm_surface_create({width}x{height}, XRGB8888, RENDERING|LINEAR) returned \
             NULL"
        ));
    }

    let egl_surface =
        unsafe { eglCreateWindowSurface(egl_display, config, gbm_surface, std::ptr::null()) };
    if egl_surface.is_null() {
        return Err(format!(
            "{node}: eglCreateWindowSurface over the GBM surface failed (EGL error {:#x})",
            unsafe { eglGetError() }
        ));
    }

    if unsafe { eglMakeCurrent(egl_display, egl_surface, egl_surface, context) } == 0 {
        return Err(format!("{node}: eglMakeCurrent failed (EGL error {:#x})", unsafe {
            eglGetError()
        }));
    }

    // SAFETY: a context is current, so `glGetString` is legal and returns a
    // pointer to a static string owned by the driver.
    let renderer = unsafe {
        let p = glGetString(GL_RENDERER);
        if p.is_null() {
            "unknown".to_string()
        } else {
            std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    };

    Ok(Screen {
        what: format!("{SOCKET}, {width}x{height}, dma-buf XRGB8888 on {renderer} (R-206)"),
        inner,
        queue,
        handle,
        _conn: conn,
        width,
        height,
        egl: Egl {
            display: egl_display,
            context,
            surface: egl_surface,
        },
        _gbm_device: gbm_device,
        uploader: None,
        gbm_surface,
        in_flight: Vec::new(),
    })
}

/// Bound `registry_queue_init` by `deadline` — B10.
///
/// The crate gives this call no timeout of its own. `registry_queue_init`
/// calls `Connection::roundtrip` internally, which polls the socket with
/// `None` (forever) and retries through `EINTR` in a loop, so not even a
/// signal breaks it out. The only lever left is the socket itself: a
/// watchdog thread shuts `watchdog` down — a second handle on the same
/// socket `conn` was built from — if `deadline` passes before
/// `registry_queue_init` returns on its own. A shutdown socket makes the
/// blocked poll return with an error instead of never returning, which is
/// what actually bounds it.
///
/// **The watchdog is disarmed before this function returns, either way.** A
/// thread left armed past that point would shut the connection down later,
/// during ordinary frame drawing, rather than during start-up — cancellation
/// is by dropping `cancel_tx`, which the standard library's
/// `mpsc::Receiver::recv_timeout` reports as `Disconnected` rather than
/// `Timeout`, and only `Timeout` arms the shutdown.
///
/// Identical to `composited::bounded_registry_init`, duplicated rather than
/// shared for the same reason `CONFIGURE_PATIENCE` is: the two modules are
/// alternatives.
fn bounded_registry_init(
    conn: &Connection,
    watchdog: std::os::unix::net::UnixStream,
    deadline: std::time::Instant,
) -> Result<(GlobalList, EventQueue<Inner>), String> {
    let (cancel_tx, cancel_rx) = std::sync::mpsc::channel::<()>();
    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
    let timer = std::thread::spawn(move || {
        if matches!(
            cancel_rx.recv_timeout(remaining),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ) {
            let _ = watchdog.shutdown(std::net::Shutdown::Both);
        }
    });

    let result = registry_queue_init::<Inner>(conn)
        .map_err(|e| format!("{SOCKET}: the compositor's globals could not be read: {e}"));

    drop(cancel_tx);
    let _ = timer.join();
    result
}

/// The first config whose `EGL_NATIVE_VISUAL_ID` is the fourcc the GBM surface
/// will be created with — see the note at the call site.
fn pick_config(display: EGLDisplay, configs: &[EGLConfig], found: EGLint) -> Option<EGLConfig> {
    let n = usize::try_from(found).unwrap_or(0).min(configs.len());
    for config in configs.get(..n)? {
        let mut visual: EGLint = 0;
        // SAFETY: `display` is initialised and `config` came from it.
        let ok = unsafe {
            eglGetConfigAttrib(display, *config, EGL_NATIVE_VISUAL_ID, &raw mut visual)
        };
        if ok != 0 && u32::from_ne_bytes(visual.to_ne_bytes()) == GBM_FORMAT_XRGB8888 {
            return Some(*config);
        }
    }
    None
}

/// Open the first render node that `gbm_create_device` will accept.
///
/// **Enumerated and not named.** `renderD128` is the answer on every board this
/// project has, and hard-coding it would be a second place the device layout is
/// written down — `rivet_core::service::device_globs` grants `renderD*` and
/// this honours the same pattern.
fn open_render_node() -> Result<(String, c_int), String> {
    use std::os::unix::io::IntoRawFd;

    let mut tried = Vec::new();
    let entries = std::fs::read_dir(DRM_NODE_DIR).map_err(|e| {
        format!(
            "{DRM_NODE_DIR}: {e} — an application reaches the GPU by declaring devices = \
             [\"gpu\"], which grants /dev/dri/renderD*"
        )
    })?;
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .filter(|n| n.starts_with("renderD"))
        .collect();
    names.sort();

    for name in &names {
        let path = format!("{DRM_NODE_DIR}/{name}");
        match std::fs::OpenOptions::new().read(true).write(true).open(&path) {
            Ok(file) => return Ok((path, file.into_raw_fd())),
            Err(e) => tried.push(format!("{path}: {e}")),
        }
    }

    Err(if tried.is_empty() {
        format!(
            "{DRM_NODE_DIR} holds no renderD* node. This machine has no GPU Rivet can reach, or \
             DRM has not probed"
        )
    } else {
        format!("no render node could be opened — {}", tried.join("; "))
    })
}

impl Screen {
    /// Make this surface's GL context current on the calling thread.
    ///
    /// [`take`] leaves it current, so an application that draws on one thread
    /// never needs this. It exists because *"the context is current"* is a
    /// per-thread fact and a silent one, and a program that gets it wrong sees
    /// GL calls do nothing rather than fail.
    ///
    /// # Errors
    ///
    /// The EGL error, named.
    pub fn make_current(&self) -> Result<(), String> {
        // SAFETY: every handle is live for `self`'s lifetime.
        if unsafe {
            eglMakeCurrent(
                self.egl.display,
                self.egl.surface,
                self.egl.surface,
                self.egl.context,
            )
        } == 0
        {
            return Err(format!("eglMakeCurrent failed (EGL error {:#x})", unsafe {
                eglGetError()
            }));
        }
        Ok(())
    }

    /// **Put the frame that was just drawn on the screen**, without it ever
    /// leaving the GPU.
    ///
    /// `eglSwapBuffers` finishes the frame into the GBM surface's front buffer;
    /// that buffer is locked and attached through its own `wl_buffer`, which
    /// is made — the buffer exported as a dma-buf — the first time the object
    /// comes round, reused on every later turn, and destroyed with the object.
    /// The compositor imports the descriptor as a texture once per object.
    /// **No pixel is copied and none crosses main memory.**
    ///
    /// # Errors
    ///
    /// Each step names itself. A failure here is a frame not shown, which an
    /// application should report and carry on from — the next one usually
    /// works, exactly as [`crate::composited::Screen::blit`] assumes.
    pub fn present(&mut self) -> Result<(), String> {
        // **Pump the connection FIRST, and give buffers back before asking for
        // one.** This used to happen at the END of this function, and that
        // made a single failed swap permanent: `present` returns early when
        // the swap fails, so nothing read the socket, no release was
        // dispatched, no buffer was reclaimed — and the next swap failed for
        // exactly the same reason. Measured on `rivet-01`: five frames, then
        // 3,789 identical failures with **three releases sitting unread**.
        //
        // The ordering rule this is an instance of: **a loop that can fail
        // must do its recovery before the thing that fails**, or the first
        // failure is the last thing that ever happens.
        self.pump()?;
        self.reclaim();

        // SAFETY: the context is current and the surface is live.
        if unsafe { eglSwapBuffers(self.egl.display, self.egl.surface) } == 0 {
            // **The counts, because the bare EGL error names the wrong
            // culprit.** `EGL_BAD_ALLOC` here means the GBM surface has no
            // free buffer, which is almost never an allocation problem — it is
            // buffers that were handed to the compositor and never given back.
            // Saying how many are held and how many releases have EVER arrived
            // separates "the compositor is not releasing" from "this client is
            // not hearing the release", which are different defects and look
            // identical from the error alone.
            return Err(format!(
                "eglSwapBuffers failed (EGL error {:#x}) — {} buffer(s) held by the compositor, \
                 {} release(s) seen in total",
                unsafe { eglGetError() },
                self.in_flight.len(),
                self.inner.releases_seen,
            ));
        }

        // SAFETY: a swap has just completed, so a front buffer exists.
        let bo = unsafe { gbm_surface_lock_front_buffer(self.gbm_surface) };
        if bo.is_null() {
            return Err(
                "gbm_surface_lock_front_buffer returned NULL — every buffer in the surface is \
                 still held by the compositor, which means releases are not being processed"
                    .to_string(),
            );
        }

        // The object's own `wl_buffer`, made on its first turn, reused on every
        // later one, and destroyed with the object — `crate::attached` says why.
        let (dmabuf, handle, width, height) =
            (&self.inner.dmabuf, &self.handle, self.width, self.height);
        // SAFETY: `bo` was locked from the surface just above, so it is live,
        // and this is the only code that sets user data on the surface's
        // objects, always an `Owned`.
        let wrapped = unsafe {
            attached::get_or_attach(
                &GbmUserData,
                bo,
                || wrap_buffer_object(dmabuf, handle, width, height, bo).map(Owned),
                |owned: &Owned| owned.0.clone(),
            )
        };
        let buffer = match wrapped {
            Ok(buffer) => buffer,
            Err(why) => {
                // SAFETY: `bo` was locked above and never reached the
                // compositor, so it goes straight back.
                unsafe { gbm_surface_release_buffer(self.gbm_surface, bo) };
                return Err(why);
            }
        };

        let surface = self.inner.window.wl_surface();
        surface.attach(Some(&buffer), 0, 0);
        surface.damage_buffer(
            0,
            0,
            i32::try_from(self.width).unwrap_or(0),
            i32::try_from(self.height).unwrap_or(0),
        );
        surface.frame(&self.handle, surface.clone());
        self.inner.window.commit();

        self.in_flight.push(InFlight { bo, buffer });

        self.pump()?;
        self.reclaim();
        Ok(())
    }

    /// **Flush what is queued, read what has arrived, dispatch it** — without
    /// waiting, because `wl_buffer::release` only arrives if something reads
    /// the socket, and without it the GBM surface runs out.
    ///
    /// A bounded round trip would also deliver the release and is what
    /// `composited::blit` uses; it is avoided here because a GPU frame must
    /// not serialise against the compositor's own render, which is the
    /// pipelining this path exists for.
    ///
    /// # Errors
    ///
    /// Only a connection that has actually gone, and then [`Screen::closed`]
    /// is true: see `composited::pump`, which this is.
    fn pump(&mut self) -> Result<(), String> {
        crate::composited::pump(&mut self.queue, &mut self.inner)
    }

    /// Give back every buffer the compositor has finished with.
    ///
    /// **The GBM surface has a small number of buffers** — at most four with
    /// Mesa — so a frame that is not reclaimed is a frame the next
    /// `lock_front_buffer` cannot allocate. `gpu-flip` hit exactly this within
    /// a few frames and its note is why this method exists rather than being
    /// inlined and forgotten.
    fn reclaim(&mut self) {
        if self.inner.released.is_empty() {
            return;
        }
        let released = std::mem::take(&mut self.inner.released);
        self.in_flight.retain(|held| {
            if released.iter().any(|id| *id == held.buffer.id()) {
                // SAFETY: the compositor has released this `wl_buffer`, so
                // nothing is reading the buffer object behind it any more.
                // The `wl_buffer` stays with the object, for the next time
                // GBM hands it out or until GBM destroys it.
                unsafe { gbm_surface_release_buffer(self.gbm_surface, held.bo) };
                false
            } else {
                true
            }
        });
    }

    /// Whether the compositor has gone away or asked this surface to close.
    #[must_use]
    pub fn closed(&self) -> bool {
        self.inner.closed
    }

    /// **Put a CPU-drawn frame on a GPU surface.**
    ///
    /// # Why a GPU screen needs this at all
    ///
    /// Because not every background draws with the GPU and the surface kind
    /// is not the background's to choose. `.rulings/` R-207 ships Digital
    /// Rain and Starfield on the CPU and Contours, Life and Boxes on the GPU,
    /// **from one pack, switchable by an operator at run time**. A host that
    /// took a `wl_shm` surface for one and a dma-buf surface for the other
    /// would have to tear down and rebuild its surface — and its overlay, and
    /// its buffer pool — every time somebody changed a picker. So the host
    /// takes ONE surface, the GPU one, and a CPU background arrives here.
    ///
    /// # What it costs, and why that is not the readback §3 warns about
    ///
    /// This is an upload: main memory to the GPU, `width * height * 4` bytes,
    /// once a frame. `GPU-Backgrounds-2026-09-20` §3's trap is the opposite
    /// direction — a GPU frame read BACK so the CPU can draw on it — and that
    /// one is expensive because a readback stalls the pipeline waiting for
    /// the GPU to finish. An upload does not stall: the driver copies and
    /// returns.
    ///
    /// **It is still not free, and a host should know the number for its own
    /// machine rather than trust this paragraph.**
    ///
    /// # The canvas must be tightly packed
    ///
    /// `width * 4` bytes per row, no padding. GLES 2.0 has no
    /// `GL_UNPACK_ROW_LENGTH` — that is ES 3.0 — so a padded canvas would
    /// have to be uploaded a row at a time, which is a different function and
    /// a worse one. [`Screen`] has no stride of its own for exactly this
    /// reason: the canvas is the application's and it should make it tight.
    ///
    /// # Errors
    ///
    /// A canvas that is not `width * height * 4` bytes, or a shader the
    /// driver would not build.
    pub fn show_pixels(
        &mut self,
        canvas: &[u8],
        width: usize,
        height: usize,
    ) -> Result<(), String> {
        let want = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| format!("a {width}x{height} surface cannot be addressed"))?;
        if canvas.len() < want {
            return Err(format!(
                "the canvas holds {} bytes and a tightly packed {width}x{height} frame is {want}",
                canvas.len()
            ));
        }
        if self.uploader.is_none() {
            self.uploader = Some(self.build_uploader()?);
        }
        let Some(uploader) = self.uploader.as_ref() else {
            return Err("the uploader was not built".to_string());
        };
        #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
        let (w, h) = (width as i32, height as i32);
        // SAFETY: the caller has made the context current — that is
        // `make_current`'s contract and this function's precondition — and
        // every object named is this struct's own. `canvas` is checked above
        // to hold the `w * h * 4` bytes `glTexImage2D` will read.
        unsafe {
            glViewport(0, 0, w, h);
            glUseProgram(uploader.program);
            // **The quad and its attribute pointers, every frame.** See
            // `Uploader::position`: a pack that drew the previous frame may
            // have bound its own buffer and left these pointing at it, and
            // the symptom would be a screen of garbage rather than an error.
            glBindBuffer(GL_ARRAY_BUFFER, uploader.quad);
            #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
            let stride = (4 * std::mem::size_of::<f32>()) as i32;
            let float = std::mem::size_of::<f32>();
            glEnableVertexAttribArray(uploader.position);
            glVertexAttribPointer(uploader.position, 2, GL_FLOAT, 0, stride, std::ptr::null());
            glEnableVertexAttribArray(uploader.uv_in);
            glVertexAttribPointer(uploader.uv_in, 2, GL_FLOAT, 0, stride, (2 * float) as *const _);
            glActiveTexture(GL_TEXTURE0);
            glBindTexture(GL_TEXTURE_2D, uploader.texture);
            // Row alignment is global state too, and a pack that uploaded an
            // 8x8 RGB palette set it to 1 or left it at 4.
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGBA as i32,
                w,
                h,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                canvas.as_ptr().cast(),
            );
            glUniform1i(uploader.u_frame, 0);
            glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        }
        Ok(())
    }

    /// Compile the one program and make the one texture and quad.
    fn build_uploader(&self) -> Result<Uploader, String> {
        // SAFETY: the context is current, and every object below is created
        // here. The two shader sources are `'static` and outlive their calls.
        unsafe {
            let compile = |kind: u32, src: &str| -> Result<u32, String> {
                let shader = glCreateShader(kind);
                let pointer = src.as_ptr().cast::<c_char>();
                #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
                let length = src.len() as i32;
                glShaderSource(shader, 1, &raw const pointer, &raw const length);
                glCompileShader(shader);
                let mut ok: i32 = 0;
                glGetShaderiv(shader, GL_COMPILE_STATUS, &raw mut ok);
                if ok == 0 {
                    return Err("the frame-upload shader would not compile".to_string());
                }
                Ok(shader)
            };
            let vertex = compile(GL_VERTEX_SHADER, UPLOAD_VERTEX_SRC)?;
            let fragment = compile(GL_FRAGMENT_SHADER, UPLOAD_FRAGMENT_SRC)?;
            let program = glCreateProgram();
            glAttachShader(program, vertex);
            glAttachShader(program, fragment);
            glLinkProgram(program);
            let mut ok: i32 = 0;
            glGetProgramiv(program, GL_LINK_STATUS, &raw mut ok);
            if ok == 0 {
                return Err("the frame-upload program would not link".to_string());
            }
            glUseProgram(program);

            let mut quad = 0u32;
            glGenBuffers(1, &raw mut quad);
            glBindBuffer(GL_ARRAY_BUFFER, quad);
            glBufferData(
                GL_ARRAY_BUFFER,
                std::mem::size_of_val(&UPLOAD_QUAD) as isize,
                UPLOAD_QUAD.as_ptr().cast(),
                GL_STATIC_DRAW,
            );
            #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
            let stride = (4 * std::mem::size_of::<f32>()) as i32;
            let float = std::mem::size_of::<f32>();
            let position = glGetAttribLocation(program, c"position".as_ptr());
            let uv_in = glGetAttribLocation(program, c"uv_in".as_ptr());
            if position < 0 || uv_in < 0 {
                return Err(
                    "the frame-upload program declares no position/uv_in pair".to_string()
                );
            }
            #[allow(clippy::cast_sign_loss)]
            let (position, uv_in) = (position as u32, uv_in as u32);
            glEnableVertexAttribArray(position);
            glVertexAttribPointer(position, 2, GL_FLOAT, 0, stride, std::ptr::null());
            glEnableVertexAttribArray(uv_in);
            glVertexAttribPointer(uv_in, 2, GL_FLOAT, 0, stride, (2 * float) as *const _);

            let mut texture = 0u32;
            glGenTextures(1, &raw mut texture);
            glBindTexture(GL_TEXTURE_2D, texture);
            // **`GL_NEAREST`, because the texture is the same size as the
            // surface.** There is nothing to interpolate, and a linear filter
            // on a one-to-one blit is a way to make text very slightly soft
            // for no reason at all.
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
            // One byte of row alignment. The default is four and a canvas of
            // odd width would skew every row after the first.
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1);

            let u_frame = glGetUniformLocation(program, c"frame".as_ptr());
            Ok(Uploader {
                program,
                texture,
                quad,
                u_frame,
                position,
                uv_in,
            })
        }
    }

    /// **Ask for a widget layer over the GPU background** — see [`Overlay`].
    ///
    /// Call once, after [`take`]. An application that draws only a background
    /// never calls it and pays nothing for it.
    ///
    /// # Errors
    ///
    /// A compositor with no `wl_shm` or no `wl_subcompositor`, or a pool it
    /// would not create. Each names which.
    pub fn take_overlay(&mut self) -> Result<(), String> {
        if self.inner.overlay.is_some() {
            return Ok(());
        }
        let overlay = Overlay::create(
            &self.inner.subcompositor,
            self.inner.window.wl_surface(),
            &self.inner.shm,
            &self.handle,
        )?;
        self.inner.overlay = Some(overlay);
        Ok(())
    }

    /// The widget layer's pixels, `width * height * 4` of **ARGB8888** — and
    /// the alpha is real here, unlike the background's padding byte.
    ///
    /// The slice is whatever was drawn last frame. An application that wants a
    /// clean layer clears it; one that redraws every pixel need not.
    ///
    /// # Errors
    ///
    /// If [`Screen::take_overlay`] has not been called, or the pool would not
    /// give a buffer.
    pub fn overlay(&mut self) -> Result<&mut [u8], String> {
        let (w, h) = (self.width, self.height);
        self.inner
            .overlay
            .as_mut()
            .ok_or_else(|| "take_overlay has not been called".to_string())?
            .canvas(w, h)
    }

    /// Put the widget layer on the screen.
    ///
    /// **Separate from [`Screen::present`] on purpose.** The background and
    /// the widgets change at wildly different rates — a plasma field every
    /// frame, a clock once a minute — and a subsurface that had to be
    /// recommitted with its parent would throw away exactly the saving a
    /// compositor exists to give.
    ///
    /// # Errors
    ///
    /// If [`Screen::take_overlay`] has not been called.
    pub fn present_overlay(&mut self) -> Result<(), String> {
        let (w, h) = (self.width, self.height);
        self.inner
            .overlay
            .as_mut()
            .ok_or_else(|| "take_overlay has not been called".to_string())?
            .present(w, h)?;
        self.pump()
    }
}

use std::os::unix::io::{AsFd, FromRawFd};

/// **The widgets' surface, drawn on the CPU and composited by the GPU.**
///
/// # The readback this exists to avoid
///
/// `GPU-Backgrounds-2026-09-20` §3 is the trap: *"a half-GPU renderer is
/// slower than an all-CPU one"*, because a frame drawn on the GPU has to come
/// **back** to main memory for the CPU to draw widgets on, and the readback of
/// 8.29 MB at 1080p costs more than the 32 ms it was trying to save. Read under
/// a DRM lease that leaves exactly two options — put everything on the GPU, or
/// nothing — and §3 sizes the first at 3,400 lines of `widgets.rs` and
/// `layout.rs` rewritten in GL.
///
/// **A compositor supplies a third, and it did not exist under the lease.**
/// The background is this surface's parent — a dma-buf the V3D rendered into
/// and never left. The widgets are a `wl_shm` **subsurface** on top of it,
/// drawn by exactly the CPU code that draws them today. `rivet-displayd`
/// blends the two on the V3D, because blending surfaces is what a compositor
/// is. **Nothing is read back and nothing is ported.**
///
/// `cmd/rivet-displayd` needs no change for this: `CompositorState::new`
/// registers `wl_subcompositor` beside `wl_compositor`, and `state.rs`'s
/// `commit` already walks to a subsurface's root.
///
/// # ARGB8888 here, XRGB8888 below
///
/// The background is opaque and its fourth byte is padding. This one is drawn
/// **over** it, so its fourth byte is alpha and it must be: a widget card at
/// `alpha 155` is how Aura keeps the rain visible through the clock
/// (`draw::fill_rounded_alpha`'s note), and a surface with no alpha would make
/// that a black rectangle.
/// **Moved to [`crate::overlay`] when `composited::Screen` wanted it too.**
/// The reasoning above is kept here because this is where a reader of the GPU
/// path will look for it; the code is one copy, in one place.
use crate::overlay::Overlay;

/// Everything the Wayland side holds, kept off [`Screen`] so an application
/// cannot reach the toolkit's handlers.
struct Inner {
    registry: RegistryState,
    outputs: OutputState,
    window: Window,
    dmabuf: ZwpLinuxDmabufV1,
    width: usize,
    height: usize,
    closed: bool,
    /// `wl_buffer`s the compositor has released since the last [`Screen::reclaim`].
    released: Vec<smithay_client_toolkit::reexports::client::backend::ObjectId>,
    /// **Every release ever seen**, for the diagnostic in `present`. A
    /// running total rather than a flag: zero is the interesting value and it
    /// has to be distinguishable from "the vector was just drained".
    releases_seen: u64,
    /// The widgets, if this application asked for them — see [`Overlay`].
    /// `None` for a background that draws no widgets, which costs it nothing.
    overlay: Option<Overlay>,
    /// Held so `take_overlay` can build a subsurface later rather than
    /// forcing every application to decide at `take` time.
    shm: Shm,
    subcompositor: SubcompositorState,
}

/// Bind `zwp_linux_dmabuf_v1`, or say plainly that this compositor has none.
fn bind_dmabuf(globals: &GlobalList, handle: &QueueHandle<Inner>) -> Result<ZwpLinuxDmabufV1, String> {
    globals.bind::<ZwpLinuxDmabufV1, Inner, _>(handle, 3..=4, ()).map_err(|e| {
        format!(
            "no zwp_linux_dmabuf_v1: {e} — this compositor cannot take a GPU buffer. That means \
             rivet-displayd predates .rulings/ R-206; an application that quietly fell back to \
             wl_shm here would be the half-GPU renderer GPU-Backgrounds-2026-09-20 §3 rules out, \
             so this refuses instead"
        )
    })
}

/// **Export `bo` as a dma-buf and wrap it in a `wl_buffer`**, once per buffer
/// object — [`Screen::present`] keeps the result in the object's GBM user data
/// and attaches it on every later turn of the same object.
///
/// # Errors
///
/// GBM could not export the object. Nothing is kept, so the object's next turn
/// tries again.
fn wrap_buffer_object(
    dmabuf: &ZwpLinuxDmabufV1,
    handle: &QueueHandle<Inner>,
    width: usize,
    height: usize,
    bo: *mut c_void,
) -> Result<wl_buffer::WlBuffer, String> {
    // SAFETY: `bo` is a live buffer object locked from the surface.
    let (fd, stride, modifier, offset) = unsafe {
        (
            gbm_bo_get_fd(bo),
            gbm_bo_get_stride(bo),
            gbm_bo_get_modifier(bo),
            gbm_bo_get_offset(bo, 0),
        )
    };
    if fd < 0 {
        return Err("gbm_bo_get_fd could not export the buffer as a dma-buf".to_string());
    }

    let params = dmabuf.create_params(handle, ());
    // SAFETY: `fd` is owned here and handed to the protocol object below.
    let owned = unsafe { std::os::unix::io::OwnedFd::from_raw_fd(fd) };
    params.add(
        owned.as_fd(),
        0,
        offset,
        stride,
        u32::try_from(modifier >> 32).unwrap_or(0),
        u32::try_from(modifier & 0xffff_ffff).unwrap_or(0),
    );
    let buffer = params.create_immed(
        i32::try_from(width).unwrap_or(0),
        i32::try_from(height).unwrap_or(0),
        GBM_FORMAT_XRGB8888,
        zwp_linux_buffer_params_v1::Flags::empty(),
        handle,
        (),
    );
    params.destroy();
    Ok(buffer)
}

impl Dispatch<ZwpLinuxDmabufV1, ()> for Inner {
    fn event(
        _state: &mut Self,
        _proxy: &ZwpLinuxDmabufV1,
        _event: <ZwpLinuxDmabufV1 as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // `format` and `modifier` events describe what the compositor accepts.
        // This module asks for one format and finds out whether it works by
        // asking for it, which is the same bargain `composited` makes with shm.
    }
}

impl Dispatch<ZwpLinuxBufferParamsV1, ()> for Inner {
    fn event(
        _state: &mut Self,
        _proxy: &ZwpLinuxBufferParamsV1,
        _event: <ZwpLinuxBufferParamsV1 as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // `created`/`failed` belong to the deferred `create` request. This
        // module uses `create_immed`, which reports failure as a protocol
        // error instead — louder, and correct for a kiosk.
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for Inner {
    /// **The release is the whole reason this impl exists.** See
    /// [`Screen::reclaim`]: a buffer object that is never given back is a
    /// surface that stops producing frames.
    fn event(
        state: &mut Self,
        proxy: &wl_buffer::WlBuffer,
        event: <wl_buffer::WlBuffer as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_buffer::Event::Release) {
            state.released.push(proxy.id());
            state.releases_seen += 1;
        }
    }
}

impl CompositorHandler for Inner {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Inner {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl crate::composited::Closes for Inner {
    fn close(&mut self) {
        self.closed = true;
    }
}

impl WindowHandler for Inner {
    fn request_close(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _window: &Window) {
        self.closed = true;
    }

    /// **A configure is the only place a size comes from.** A GBM surface is
    /// created against it once; a resize after that is not handled here, and
    /// on a kiosk whose output never changes mode there is nothing to handle.
    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _window: &Window,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        let (w, h) = configure.new_size;
        if let (Some(w), Some(h)) = (w, h) {
            self.width = usize::try_from(w.get()).unwrap_or(0);
            self.height = usize::try_from(h.get()).unwrap_or(0);
        }
    }
}

impl ProvidesRegistryState for Inner {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState];
}

impl ShmHandler for Inner {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_shm!(Inner);
delegate_subcompositor!(Inner);
delegate_compositor!(Inner);
delegate_output!(Inner);
delegate_xdg_shell!(Inner);
delegate_xdg_window!(Inner);
delegate_registry!(Inner);
crate::composited::answers_sync!(Inner);

#[cfg(test)]
mod tests {
    //! B10: the compositor's first configure has a ten-second deadline; these
    //! prove it is actually enforced rather than merely checked around a
    //! call that can itself block forever. Both peers here are ends of a
    //! `UnixStream::pair()` the test holds open and never writes to — a
    //! real "accepts the connection and then says nothing" peer, not a
    //! model of one.
    //!
    //! Neither test touches GBM, EGL or GL — `bounded_registry_init` and
    //! `composited::bounded_dispatch`, which this module's configure wait
    //! shares, are pure Wayland-socket code, and exercising them needs no
    //! render node.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// Every silent-peer test races the call under test against a bound of
    /// its own, well clear of the short `deadline` actually being tested —
    /// so that on the pre-B10 code, which can block forever, the test FAILS
    /// by timing out this harness rather than hanging the whole suite.
    const HARNESS_BOUND: std::time::Duration = std::time::Duration::from_secs(5);

    /// B10, registry phase: a peer that accepts the connection and never
    /// sends a byte must not hang `bounded_registry_init` past `deadline`.
    ///
    /// Before B10, `take` called `registry_queue_init` directly here — no
    /// watchdog, no bound — and a silent peer left it blocked forever inside
    /// `Connection::roundtrip`'s own `poll(None)`.
    #[test]
    fn registry_discovery_silence_is_bounded() {
        let (client, peer) = std::os::unix::net::UnixStream::pair().expect("socket pair");
        let watchdog = client.try_clone().expect("clone the client socket");
        let conn = Connection::from_socket(client).expect("wrap the client socket");
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);

        let (tx, rx) = std::sync::mpsc::channel();
        let started = std::time::Instant::now();
        std::thread::spawn(move || {
            let _peer = peer; // held open and silent for the life of the call
            let result = bounded_registry_init(&conn, watchdog, deadline);
            let _ = tx.send(result);
        });

        let result = rx.recv_timeout(HARNESS_BOUND).unwrap_or_else(|_| {
            panic!(
                "bounded_registry_init did not return within the harness's own {HARNESS_BOUND:?} \
                 bound against a silent peer — this is the B10 regression: an unbounded wait"
            )
        });
        let elapsed = started.elapsed();

        let err = result.expect_err("a silent peer must not be reported as successful");
        assert!(
            err.contains("globals could not be read"),
            "expected the registry-discovery error message, got: {err}"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "took {elapsed:?} against a 200ms deadline — bounded, but not by the deadline"
        );
    }

    /// B10, configure phase: a peer that answers nothing at all must not
    /// hang `bounded_dispatch` past `deadline` either — the same call
    /// `take`'s configure-wait loop makes every iteration, tested here with
    /// `State = ()` rather than a real `Inner` so the peer needs no
    /// protocol knowledge to be a faithful stand-in for "the registry
    /// answered, and then the compositor went quiet before the configure."
    ///
    /// Before B10 the loop called `EventQueue::blocking_dispatch` directly —
    /// no bound on the read inside it — so a silent peer left it blocked
    /// forever, between deadline checks that never got the chance to run
    /// again.
    #[test]
    fn configure_wait_silence_is_bounded() {
        let (client, peer) = std::os::unix::net::UnixStream::pair().expect("socket pair");
        let conn = Connection::from_socket(client).expect("wrap the client socket");
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);

        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let started = std::time::Instant::now();
        std::thread::spawn(move || {
            let _peer = peer;
            let mut queue = conn.new_event_queue::<()>();
            let mut data = ();
            // The exact shape of `take`'s own loop: check the deadline, then
            // make one bounded dispatch call.
            loop {
                if std::time::Instant::now() > deadline {
                    let _ = tx.send(Err("timed out, as expected".to_string()));
                    return;
                }
                if let Err(e) = crate::composited::bounded_dispatch(&mut queue, &mut data, deadline) {
                    let _ = tx.send(Err(e));
                    return;
                }
            }
        });

        let result = rx.recv_timeout(HARNESS_BOUND).unwrap_or_else(|_| {
            panic!(
                "the configure-wait loop did not return within the harness's own \
                 {HARNESS_BOUND:?} bound against a silent peer — this is the B10 regression"
            )
        });
        let elapsed = started.elapsed();

        assert_eq!(result, Err("timed out, as expected".to_string()));
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "took {elapsed:?} against a 200ms deadline — bounded, but not by the deadline"
        );
    }
}
