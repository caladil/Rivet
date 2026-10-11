//! **Does the GPU actually run?** — `PLAN.md` §104.17.
//!
//! `§104.13` put VC4's KMS driver on a board and `§104.12` explained why it had
//! not bound. What none of that established is whether anything can *render*:
//! `/dev/dri/renderD128` existing is a node, and a node is not a shader.
//!
//! So this program does the smallest thing that cannot be faked. It builds an
//! EGL context on the V3D render node through GBM, compiles a vertex and a
//! fragment shader, draws a spinning triangle into an off-screen buffer, reads
//! the pixels back and — **only when nothing else could be holding the screen**
//! — puts them on `/dev/fb0`. See [`allowed_to_draw`]: under a supervisor it
//! renders and reports and touches no framebuffer, which is where the answer
//! lives anyway.
//!
//! # The one line that is the whole proof
//!
//! ```text
//! mesa-demo: GL_RENDERER = V3D 4.2
//! ```
//!
//! **`glGetString(GL_RENDERER)` is what says whether the GPU ran.** Mesa will
//! happily produce a working context on `llvmpipe`, its software rasteriser,
//! and every other line of output would look identical — the triangle spins,
//! the colours interpolate, nothing errors. A demo that did not print this
//! would prove that *Mesa* works and say nothing about the *hardware*, which is
//! the question. `AGENTS.md` §5: report what the system said, and pick the
//! thing it says that can distinguish the two answers.
//!
//! # Off-screen and then blitted, rather than KMS page-flipping
//!
//! **Deliberately the smaller half.** Full KMS — enumerate connectors, pick a
//! mode, `drmModeSetCrtc`, allocate scanout buffers, page-flip on vblank — is
//! several hundred lines of `libdrm` before a single triangle appears, and
//! every one of them is a place for a first attempt to fail for a reason that
//! has nothing to do with the GPU.
//!
//! Rendering off-screen and copying to the framebuffer reuses a path
//! `kiosk-demo` already proved and isolates the question to *"can this machine
//! compile and run a shader"*. It is slower and it tears, and neither matters
//! for an answer. **Page-flipping is what Aura will need** and is the next
//! piece, not this one.
//!
//! # No crates, on `kiosk-demo`'s argument
//!
//! `ADR-009` §5 asks that every dependency be justified, and an example whose
//! subject is *whether the driver works* should not answer it through three
//! layers of somebody else's bindings — a failure in `gbm-rs` and a failure in
//! `gbm` would look the same from here. The declarations below are the exact
//! set this program calls and nothing else.

use std::ffi::{CString, c_char, c_int, c_void};

/// The render node. **Not `card0`.** A render node needs no modesetting rights
/// and cannot disturb whatever owns the screen, which is the right thing for a
/// program that only wants to run shaders.
const RENDER_NODE: &str = "/dev/dri/renderD128";

/// Where the result is shown.
const FB_DEV: &str = "/dev/fb0";
const FB_SYS: &str = "/sys/class/graphics/fb0";

/// The channel a supervisor sends display leases on — `rivet_sdk::display`'s
/// `DISPLAY_ENV`, spelled out here because this program declares what it uses
/// rather than depending on it (see the module header's argument about crates).
///
/// **Its presence or absence is the whole of [`allowed_to_draw`].**
const DISPLAY_ENV: &str = "RIVET_DISPLAY";

/// The directory a supervisor hands every application it starts —
/// `rivet_sdk::app`'s `APP_DATA_ENV`.
///
/// Used here only as the answer to *"was this started by a supervisor at all"*.
const APP_DATA_ENV: &str = "RIVET_APP_DATA";

/// **May this program put pixels on `/dev/fb0`?**
///
/// `ADR-036` §3.1, register `R-33`. This probe used to open the framebuffer
/// unconditionally, which is wrong in the two cases that matter and right only
/// in the one it was written in:
///
/// | `RIVET_APP_DATA` | `RIVET_DISPLAY` | What that means | What this does |
/// | :-- | :-- | :-- | :-- |
/// | set | absent | a supervisor started this and did **not** grant the display; the launcher is holding the menu | render, report, **do not touch the framebuffer** |
/// | set | set | a lease was cut for this program | render, report, **do not touch the framebuffer** — see below |
/// | absent | absent | run by hand on a board with no launcher | draw, as it always did |
///
/// **The second row is the surprising one and it is not laziness.** A lease is
/// a DRM object; `/dev/fb0` is a different device that scans out the same
/// panel, and writing it goes *around* the lease rather than through it. A
/// program handed a lease that then wrote the framebuffer would be drawing over
/// whatever the lease's owner — itself — had put there, which is exactly the
/// compatibility path `ADR-036` §3.1 says an application does not get. This
/// probe renders off-screen and blits, and consuming a lease properly means the
/// KMS page-flip path the module header defers; until it has that, the honest
/// thing is to answer its own question (`GL_RENDERER`) and leave the screen
/// alone.
///
/// **The GPU question is answered in every row.** Nothing above changes whether
/// EGL comes up, whether the shaders compile, or what `glGetString` says — which
/// is the only thing this program exists to find out.
///
/// # The kernel now agrees with the last two rows — 2026-09-04
///
/// This check was the only thing stopping the write, because the display grant
/// chowned `/dev/fb0` to every application that declared `display`. It no
/// longer does: a lessee's grant is `/dev/dri/renderD*` alone (`ADR-036` §3.1,
/// `.rulings/` R-138), so the framebuffer is `root:root` and the open would
/// fail whatever this function said.
///
/// **It is kept, and the reason is the one `AGENTS.md` §3 gives for ranking
/// gates.** The kernel's refusal is the strong one and cannot explain itself; a
/// person reading `EACCES` on `/dev/fb0` learns that a permission is missing,
/// not that this machine hands displays out on a socket. This says the sentence.
/// It also keeps the probe honest where the kernel has nothing to say — the
/// `(true, true)` row, where a lease *was* cut and writing the framebuffer would
/// be this program drawing around its own lease.
fn allowed_to_draw() -> Result<(), String> {
    let supervised = std::env::var_os(APP_DATA_ENV).is_some();
    let handed = std::env::var_os(DISPLAY_ENV).is_some();
    match (supervised, handed) {
        (false, _) => Ok(()),
        (true, false) => Err(format!(
            "a supervisor started this and {DISPLAY_ENV} is not set, so the display was not \
             granted to it — something else is holding the screen and writing {FB_DEV} would \
             draw over it. The kernel refuses it too: a lessee's display grant covers \
             /dev/dri/renderD* only, so {FB_DEV} is still root:root (ADR-036 §3.1)"
        )),
        (true, true) => Err(format!(
            "{DISPLAY_ENV} is set, so a LEASE was cut for this program — and {FB_DEV} is a \
             different device onto the same panel, so writing it would go around that lease \
             rather than through it. Driving a lease needs the KMS page-flip path this probe \
             deliberately does not have"
        )),
    }
}

fn main() {
    println!("mesa-demo: pid {}", std::process::id());

    // **Asked before anything is opened**, so the refusal is a sentence rather
    // than a screen somebody has to walk into a room to see.
    let draw_to_screen = match allowed_to_draw() {
        Ok(()) => true,
        Err(why) => {
            println!("mesa-demo: NOT drawing to {FB_DEV}: {why}");
            println!(
                "mesa-demo: the GPU question is answered either way — GL_RENDERER is below and \
                 is the only thing this probe exists to find out"
            );
            false
        }
    };

    let screen = match geometry() {
        Ok(s) => {
            println!(
                "mesa-demo: {FB_DEV} is {}x{}, {} bits per pixel, stride {}",
                s.width, s.height, s.bits, s.stride
            );
            s
        }
        Err(why) => {
            println!("mesa-demo: cannot read the framebuffer's shape: {why}");
            // **Only fatal when this program was going to draw** — 2026-09-04.
            //
            // It used to be fatal always, and that made the whole probe depend
            // on a framebuffer it is not allowed to touch under a supervisor.
            // Since the display grant was narrowed to the render node
            // (`ADR-036` §3.1), `/dev/fb0` is a device this program can never
            // open while something else holds the screen — so giving up on
            // `GL_RENDERER` because its shape could not be read would be
            // abandoning the only question this exists to answer, for a reason
            // that no longer has anything to do with it.
            if draw_to_screen {
                return idle("there is no screen to draw on");
            }
            println!(
                "mesa-demo: rendering off screen at {}x{} instead. The size is arbitrary: \
                 nothing sees these pixels and GL_RENDERER does not depend on them",
                OFFSCREEN.width, OFFSCREEN.height
            );
            OFFSCREEN
        }
    };

    let gpu = match Gpu::open() {
        Ok(g) => g,
        Err(why) => {
            println!("mesa-demo: NO GPU CONTEXT: {why}");
            return idle("the GPU could not be opened");
        }
    };

    // **The answer, printed before anything is drawn.** If this says `llvmpipe`
    // the rest of the program still works and proves nothing about hardware.
    println!("mesa-demo: GL_VENDOR   = {}", gpu.string(GL_VENDOR));
    println!("mesa-demo: GL_RENDERER = {}", gpu.string(GL_RENDERER));
    println!("mesa-demo: GL_VERSION  = {}", gpu.string(GL_VERSION));
    if gpu.string(GL_RENDERER).contains("llvmpipe") || gpu.string(GL_RENDERER).contains("softpipe")
    {
        println!(
            "mesa-demo: THAT IS A SOFTWARE RASTERISER. Mesa works and the GPU is not being \
             used — the V3D driver did not load. Everything below will still draw."
        );
    }

    let mut fb = if draw_to_screen {
        match std::fs::OpenOptions::new().read(true).write(true).open(FB_DEV) {
            Ok(f) => Some(f),
            Err(e) => {
                println!("mesa-demo: could NOT open {FB_DEV}: {e}");
                println!(
                    "mesa-demo: that is the default-deny working. An application gets this node \
                     only by declaring `devices = [\"display\"]` (ADR-015 §6.2)"
                );
                return idle("the display grant was not given");
            }
        }
    } else {
        // Not opened at all, rather than opened and not written. A descriptor
        // on the framebuffer is a thing a later edit can write through, and the
        // whole finding here is that one did.
        None
    };

    if let Err(why) = gpu.prepare(screen.width, screen.height) {
        println!("mesa-demo: the render target or the shaders were refused: {why}");
        return idle("nothing can be drawn");
    }
    println!("mesa-demo: shaders compiled and linked — drawing");

    let mut frame: u64 = 0;
    let mut pixels = vec![0u8; screen.width * screen.height * 4];
    let mut out = vec![0u8; screen.stride * screen.height];
    loop {
        let angle = (frame % 360) as f32 * std::f32::consts::PI / 180.0;
        gpu.draw(angle, screen.width, screen.height, &mut pixels);
        blit(&pixels, &mut out, &screen);
        if let Some(fb) = fb.as_mut() {
            if let Err(e) = write_frame(fb, &out) {
                println!("mesa-demo: the screen stopped accepting bytes: {e}");
                return idle("the framebuffer went away");
            }
        }
        if frame % 300 == 0 {
            println!(
                "mesa-demo: frame {frame} — still rendering on {}{}",
                gpu.string(GL_RENDERER),
                if fb.is_some() { "" } else { ", off screen" }
            );
        }
        frame += 1;
        // ~20 fps. Fast enough to look like motion, slow enough that a
        // read-back-and-copy loop does not spend the whole machine.
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// **The three rows of [`allowed_to_draw`]'s table, asserted.**
    ///
    /// Register `R-33`. This probe wrote `/dev/fb0` unconditionally, so an
    /// operator who selected it from the launcher's menu — or who merely had it
    /// installed and enabled beside something else — got a spinning triangle
    /// over whatever was on the screen, with nothing anywhere saying why.
    ///
    /// The environment is process-wide, so these run in one test rather than
    /// three: two tests setting `RIVET_DISPLAY` in parallel would see each
    /// other's value and the failure would be intermittent.
    #[test]
    fn the_framebuffer_is_written_only_when_nothing_else_could_be_holding_it() {
        // SAFETY: `set_var`/`remove_var` are unsafe because another thread may
        // be reading the environment. This test process has one thread doing
        // so — this one — because every case is checked here rather than in a
        // sibling test the harness would run in parallel.
        unsafe {
            std::env::remove_var(APP_DATA_ENV);
            std::env::remove_var(DISPLAY_ENV);
        }
        assert!(
            allowed_to_draw().is_ok(),
            "run by hand on a board with no launcher, drawing is the correct thing"
        );

        unsafe { std::env::set_var(APP_DATA_ENV, "/state/app/mesa-demo/data") };
        let why = allowed_to_draw().expect_err("a supervisor started this and granted no display");
        assert!(why.contains(DISPLAY_ENV), "it names the variable: {why}");
        assert!(why.contains("ADR-036"), "it cites the rule: {why}");
        // **What the grant actually covers, in the sentence** — 2026-09-04.
        // The refusal used to rest on the framebuffer being this application's
        // to write, which stopped being true when the grant was narrowed
        // (`.rulings/` R-138). A sentence explaining itself with a fact that has
        // rotted sends an operator to the wrong place, so the fact is asserted.
        assert!(
            why.contains("/dev/dri/renderD*"),
            "it says what a lessee's display grant covers: {why}"
        );

        unsafe { std::env::set_var(DISPLAY_ENV, "3") };
        let why = allowed_to_draw().expect_err("a lease is not a licence to write fb0");
        assert!(why.contains("LEASE"), "it says what it was handed: {why}");
        assert!(why.contains(FB_DEV), "it names the device it is refusing: {why}");

        unsafe {
            std::env::remove_var(APP_DATA_ENV);
            std::env::remove_var(DISPLAY_ENV);
        }
    }
}

/// Report and then stay alive.
///
/// **`kiosk-demo`'s rule.** A program that exited on failure would be restarted
/// by the supervisor forever and the journal would fill with starts rather than
/// with the reason. Staying up with one clear line is what an operator can act
/// on.
fn idle(why: &str) -> ! {
    println!("mesa-demo: idle — {why}");
    loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
}

// ---------------------------------------------------------------------------
// The framebuffer
// ---------------------------------------------------------------------------

struct Screen {
    width: usize,
    height: usize,
    stride: usize,
    bits: usize,
}

/// **What to render at when there is no framebuffer to take a size from.**
///
/// Reached only when this program is not going to draw — see `main`. Small on
/// purpose: the read-back is `width * height * 4` bytes a frame and nothing
/// looks at them, so a 1080p off-screen target would spend the machine to
/// produce a number that does not change with resolution.
const OFFSCREEN: Screen = Screen {
    width: 640,
    height: 480,
    stride: 640 * 4,
    bits: 32,
};

/// A stride that did not cover a row would make `blit` index past the end of
/// its output buffer, which is a panic rather than a wrong picture. Checked at
/// COMPILE time because the constant is the only thing that could break it, and
/// a runtime test of a constant is a test that can be deleted with the code it
/// guards still shipping.
const _: () = assert!(
    OFFSCREEN.stride >= OFFSCREEN.width * (OFFSCREEN.bits / 8),
    "the off-screen render target's stride does not cover one row at its own depth"
);

/// Read the shape from `sysfs`, exactly as `rivet-splash` does and for the same
/// reason: the Pi's framebuffer is whatever the display negotiated, and a
/// program that assumed 1920 would draw off the edge of every line.
fn geometry() -> Result<Screen, String> {
    let read = |name: &str| -> Result<String, String> {
        std::fs::read_to_string(format!("{FB_SYS}/{name}"))
            .map(|s| s.trim().to_string())
            .map_err(|e| format!("{FB_SYS}/{name}: {e}"))
    };
    let size = read("virtual_size")?;
    let (w, h) = size
        .split_once(',')
        .ok_or_else(|| format!("virtual_size is {size:?}, not `width,height`"))?;
    Ok(Screen {
        width: w.parse().map_err(|_| format!("width {w:?}"))?,
        height: h.parse().map_err(|_| format!("height {h:?}"))?,
        stride: read("stride")?.parse().map_err(|_| "stride".to_string())?,
        bits: read("bits_per_pixel")?
            .parse()
            .map_err(|_| "bits_per_pixel".to_string())?,
    })
}

/// RGBA8 from the GPU into whatever the framebuffer wants.
///
/// **The GPU is read back as RGBA8 and converted here**, rather than asked for
/// the framebuffer's format directly. GLES2 guarantees exactly one
/// `glReadPixels` combination — `GL_RGBA`/`GL_UNSIGNED_BYTE` — and anything
/// else is implementation-defined and has to be queried. Converting five lines
/// here is cheaper than a query that can answer *no*.
///
/// **And the rows are flipped.** OpenGL's origin is bottom-left and a
/// framebuffer's is top-left, so a copy that did not invert would draw the
/// scene upside down — which looks like a rendering bug and is a convention.
fn blit(pixels: &[u8], out: &mut [u8], s: &Screen) {
    for y in 0..s.height {
        let src = (s.height - 1 - y) * s.width * 4;
        let dst = y * s.stride;
        for x in 0..s.width {
            let p = src + x * 4;
            let (r, g, b) = (pixels[p], pixels[p + 1], pixels[p + 2]);
            match s.bits {
                16 => {
                    let at = dst + x * 2;
                    let packed = (u16::from(r >> 3) << 11)
                        | (u16::from(g >> 2) << 5)
                        | u16::from(b >> 3);
                    out[at] = (packed & 0xff) as u8;
                    out[at + 1] = (packed >> 8) as u8;
                }
                _ => {
                    let at = dst + x * (s.bits / 8);
                    // Little-endian XRGB, which is what a Pi presents at 32.
                    out[at] = b;
                    out[at + 1] = g;
                    out[at + 2] = r;
                }
            }
        }
    }
}

fn write_frame(fb: &mut std::fs::File, out: &[u8]) -> std::io::Result<()> {
    use std::io::{Seek, SeekFrom, Write};
    fb.seek(SeekFrom::Start(0))?;
    fb.write_all(out)
}

// ---------------------------------------------------------------------------
// EGL and GLES2, declared rather than depended on
// ---------------------------------------------------------------------------

type EGLDisplay = *mut c_void;
type EGLConfig = *mut c_void;
type EGLContext = *mut c_void;
type EGLSurface = *mut c_void;
type EGLint = i32;

const EGL_PLATFORM_GBM_KHR: u32 = 0x31D7;
const EGL_NONE: EGLint = 0x3038;
const EGL_OPENGL_ES_API: u32 = 0x30A0;
const EGL_CONTEXT_CLIENT_VERSION: EGLint = 0x3098;
const EGL_SURFACE_TYPE: EGLint = 0x3033;
const EGL_PBUFFER_BIT: EGLint = 0x0001;
const EGL_RENDERABLE_TYPE: EGLint = 0x3040;
const EGL_OPENGL_ES2_BIT: EGLint = 0x0004;
const EGL_RED_SIZE: EGLint = 0x3024;
const EGL_GREEN_SIZE: EGLint = 0x3023;
const EGL_BLUE_SIZE: EGLint = 0x3022;

const GL_VENDOR: u32 = 0x1F00;
const GL_RENDERER: u32 = 0x1F01;
const GL_VERSION: u32 = 0x1F02;
const GL_FRAMEBUFFER: u32 = 0x8D40;
const GL_RENDERBUFFER: u32 = 0x8D41;
const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
const GL_RGBA8: u32 = 0x8058;
const GL_FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
const GL_COLOR_BUFFER_BIT: u32 = 0x4000;
const GL_VERTEX_SHADER: u32 = 0x8B31;
const GL_FRAGMENT_SHADER: u32 = 0x8B30;
const GL_COMPILE_STATUS: u32 = 0x8B81;
const GL_LINK_STATUS: u32 = 0x8B82;
const GL_FLOAT: u32 = 0x1406;
const GL_TRIANGLES: u32 = 0x0004;
const GL_RGBA: u32 = 0x1908;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_FALSE: u8 = 0;

#[link(name = "gbm")]
unsafe extern "C" {
    fn gbm_create_device(fd: c_int) -> *mut c_void;
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
        num: *mut EGLint,
    ) -> u32;
    fn eglCreateContext(
        dpy: EGLDisplay,
        config: EGLConfig,
        share: EGLContext,
        attrib: *const EGLint,
    ) -> EGLContext;
    fn eglMakeCurrent(
        dpy: EGLDisplay,
        draw: EGLSurface,
        read: EGLSurface,
        ctx: EGLContext,
    ) -> u32;
    fn eglGetError() -> EGLint;
}

#[link(name = "GLESv2")]
unsafe extern "C" {
    fn glGetString(name: u32) -> *const c_char;
    fn glGenFramebuffers(n: i32, out: *mut u32);
    fn glBindFramebuffer(target: u32, fb: u32);
    fn glGenRenderbuffers(n: i32, out: *mut u32);
    fn glBindRenderbuffer(target: u32, rb: u32);
    fn glRenderbufferStorage(target: u32, format: u32, w: i32, h: i32);
    fn glFramebufferRenderbuffer(target: u32, attach: u32, rbtarget: u32, rb: u32);
    fn glCheckFramebufferStatus(target: u32) -> u32;
    fn glViewport(x: i32, y: i32, w: i32, h: i32);
    fn glClearColor(r: f32, g: f32, b: f32, a: f32);
    fn glClear(mask: u32);
    fn glCreateShader(kind: u32) -> u32;
    fn glShaderSource(sh: u32, count: i32, src: *const *const c_char, len: *const i32);
    fn glCompileShader(sh: u32);
    fn glGetShaderiv(sh: u32, pname: u32, out: *mut i32);
    fn glGetShaderInfoLog(sh: u32, max: i32, len: *mut i32, log: *mut c_char);
    fn glCreateProgram() -> u32;
    fn glAttachShader(prog: u32, sh: u32);
    fn glLinkProgram(prog: u32);
    fn glGetProgramiv(prog: u32, pname: u32, out: *mut i32);
    fn glUseProgram(prog: u32);
    fn glGetAttribLocation(prog: u32, name: *const c_char) -> i32;
    fn glGetUniformLocation(prog: u32, name: *const c_char) -> i32;
    fn glUniform1f(loc: i32, v: f32);
    fn glVertexAttribPointer(
        index: u32,
        size: i32,
        kind: u32,
        normalized: u8,
        stride: i32,
        ptr: *const c_void,
    );
    fn glEnableVertexAttribArray(index: u32);
    fn glDrawArrays(mode: u32, first: i32, count: i32);
    fn glReadPixels(x: i32, y: i32, w: i32, h: i32, fmt: u32, kind: u32, out: *mut c_void);
    fn glFinish();
}

const VERTEX_SHADER: &str = r"
attribute vec2 position;
attribute vec3 colour;
uniform float angle;
varying vec3 shade;
void main() {
    float c = cos(angle);
    float s = sin(angle);
    vec2 turned = vec2(position.x * c - position.y * s,
                       position.x * s + position.y * c);
    gl_Position = vec4(turned, 0.0, 1.0);
    shade = colour;
}
";

const FRAGMENT_SHADER: &str = r"
precision mediump float;
varying vec3 shade;
void main() {
    gl_FragColor = vec4(shade, 1.0);
}
";

/// One EGL context on the render node, and the program drawn with it.
struct Gpu {
    dpy: EGLDisplay,
    program: std::cell::Cell<u32>,
    angle_loc: std::cell::Cell<i32>,
}

impl Gpu {
    /// Build a context, or say which step refused.
    fn open() -> Result<Self, String> {
        use std::os::fd::AsRawFd;
        let node = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(RENDER_NODE)
            .map_err(|e| {
                format!(
                    "{RENDER_NODE}: {e} — an application gets this node only by declaring \
                     `devices = [\"display\"]` (ADR-015 §6.2). Since 2026-09-04 that is ALL a \
                     lessee's display grant covers: /dev/dri/card* and /dev/fb0 are doors \
                     around the lease it is handed and are no longer granted (ADR-036 §3.1)"
                )
            })?;
        println!(
            "mesa-demo: OPENED {RENDER_NODE} — the render node is the whole of a lessee's \
             display grant, and it is exactly what this probe needs"
        );

        // SAFETY: every call below is a C function taking the arguments its
        // header declares. `node` owns the descriptor for the whole of `open`
        // and `gbm_create_device` does not take ownership of it; the device
        // outlives this function only through `dpy`, which is never closed
        // because this program runs until the machine stops.
        unsafe {
            let gbm = gbm_create_device(node.as_raw_fd());
            if gbm.is_null() {
                return Err("gbm_create_device returned NULL — no GBM backend for this node"
                    .to_string());
            }
            // The descriptor must outlive the GBM device, and this program's
            // lifetime is the machine's.
            std::mem::forget(node);

            let dpy = eglGetPlatformDisplay(EGL_PLATFORM_GBM_KHR, gbm, std::ptr::null());
            if dpy.is_null() {
                return Err(format!("eglGetPlatformDisplay: EGL error {:#x}", eglGetError()));
            }
            let (mut major, mut minor) = (0, 0);
            if eglInitialize(dpy, &raw mut major, &raw mut minor) == 0 {
                return Err(format!("eglInitialize: EGL error {:#x}", eglGetError()));
            }
            println!("mesa-demo: EGL {major}.{minor}");
            if eglBindAPI(EGL_OPENGL_ES_API) == 0 {
                return Err("eglBindAPI(OPENGL_ES) refused".to_string());
            }

            // **No `EGL_SURFACE_TYPE`, and asking for one was the bug.** The
            // first version asked for `EGL_PBUFFER_BIT` and the board answered
            // `eglChooseConfig found no ES2 configuration`: a GBM display
            // offers WINDOW configs, because a GBM surface is what one draws
            // to. This context has no surface at all -- everything goes to a
            // renderbuffer -- so constraining the surface type asked for a
            // capability nothing here uses and rejected every config that had
            // the capability it does.
            let attrib = [
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
            let mut config: EGLConfig = std::ptr::null_mut();
            let mut found: EGLint = 0;
            if eglChooseConfig(dpy, attrib.as_ptr(), &raw mut config, 1, &raw mut found) == 0
                || found == 0
            {
                return Err("eglChooseConfig found no ES2 configuration".to_string());
            }

            let ctx_attrib = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
            let ctx = eglCreateContext(dpy, config, std::ptr::null_mut(), ctx_attrib.as_ptr());
            if ctx.is_null() {
                return Err(format!("eglCreateContext: EGL error {:#x}", eglGetError()));
            }
            // **Surfaceless.** There is no window and no scanout buffer; the
            // whole scene goes to a renderbuffer this program owns.
            if eglMakeCurrent(dpy, std::ptr::null_mut(), std::ptr::null_mut(), ctx) == 0 {
                return Err(format!(
                    "eglMakeCurrent with no surface: EGL error {:#x} — the driver may lack \
                     EGL_KHR_surfaceless_context",
                    eglGetError()
                ));
            }
            Ok(Self {
                dpy,
                program: std::cell::Cell::new(0),
                angle_loc: std::cell::Cell::new(-1),
            })
        }
    }

    /// One of the driver's identifying strings.
    fn string(&self, name: u32) -> String {
        let _ = self.dpy;
        // SAFETY: `glGetString` returns a pointer to a static NUL-terminated
        // string owned by the driver, valid for the life of the context.
        unsafe {
            let p = glGetString(name);
            if p.is_null() {
                return "<null>".to_string();
            }
            std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }

    /// The render target and the shaders.
    fn prepare(&self, width: usize, height: usize) -> Result<(), String> {
        // SAFETY: a current context exists (`open` made one) and every call is
        // the GLES2 entry point its header declares.
        unsafe {
            let mut fbo = 0u32;
            let mut rbo = 0u32;
            glGenFramebuffers(1, &raw mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glGenRenderbuffers(1, &raw mut rbo);
            glBindRenderbuffer(GL_RENDERBUFFER, rbo);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, width as i32, height as i32);
            glFramebufferRenderbuffer(
                GL_FRAMEBUFFER,
                GL_COLOR_ATTACHMENT0,
                GL_RENDERBUFFER,
                rbo,
            );
            let status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
            if status != GL_FRAMEBUFFER_COMPLETE {
                return Err(format!(
                    "the off-screen target is incomplete: status {status:#x} at {width}x{height}"
                ));
            }
            glViewport(0, 0, width as i32, height as i32);

            let vs = compile(GL_VERTEX_SHADER, VERTEX_SHADER)?;
            let fs = compile(GL_FRAGMENT_SHADER, FRAGMENT_SHADER)?;
            let program = glCreateProgram();
            glAttachShader(program, vs);
            glAttachShader(program, fs);
            glLinkProgram(program);
            let mut ok = 0i32;
            glGetProgramiv(program, GL_LINK_STATUS, &raw mut ok);
            if ok == 0 {
                return Err("the shader program did not link".to_string());
            }
            glUseProgram(program);
            self.program.set(program);
            let name = CString::new("angle").map_err(|e| e.to_string())?;
            self.angle_loc
                .set(glGetUniformLocation(program, name.as_ptr()));
            Ok(())
        }
    }

    /// One frame, read back into `pixels`.
    fn draw(&self, angle: f32, width: usize, height: usize, pixels: &mut [u8]) {
        // A triangle with a colour at each corner, so the fragment shader has
        // something to interpolate — a flat triangle would draw the same on a
        // GPU that ignored `varying`.
        const VERTICES: [f32; 6] = [0.0, 0.6, -0.6, -0.5, 0.6, -0.5];
        const COLOURS: [f32; 9] = [1.0, 0.35, 0.1, 0.1, 0.8, 0.9, 0.6, 0.9, 0.2];

        // SAFETY: a current context and a linked program exist, and every
        // pointer below is to a live local or to `pixels`, which is large
        // enough for `width * height * 4` bytes by construction in `main`.
        unsafe {
            glClearColor(0.05, 0.07, 0.09, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            glUseProgram(self.program.get());
            glUniform1f(self.angle_loc.get(), angle);

            let pos = CString::new("position").unwrap_or_default();
            let col = CString::new("colour").unwrap_or_default();
            let pos_loc = glGetAttribLocation(self.program.get(), pos.as_ptr());
            let col_loc = glGetAttribLocation(self.program.get(), col.as_ptr());
            if pos_loc >= 0 {
                let i = pos_loc as u32;
                glVertexAttribPointer(i, 2, GL_FLOAT, GL_FALSE, 0, VERTICES.as_ptr().cast());
                glEnableVertexAttribArray(i);
            }
            if col_loc >= 0 {
                let i = col_loc as u32;
                glVertexAttribPointer(i, 3, GL_FLOAT, GL_FALSE, 0, COLOURS.as_ptr().cast());
                glEnableVertexAttribArray(i);
            }
            glDrawArrays(GL_TRIANGLES, 0, 3);
            glFinish();
            glReadPixels(
                0,
                0,
                width as i32,
                height as i32,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                pixels.as_mut_ptr().cast(),
            );
        }
    }
}

/// Compile one shader, or return what the driver said was wrong with it.
///
/// **The info log travels.** A shader that does not compile is a program that
/// draws nothing, and *"the vertex shader did not compile"* without the
/// driver's own message is a sentence nobody can act on.
///
/// # Safety
///
/// A current GLES2 context must exist.
unsafe fn compile(kind: u32, source: &str) -> Result<u32, String> {
    unsafe {
        let sh = glCreateShader(kind);
        let src = CString::new(source).map_err(|e| e.to_string())?;
        let ptr = src.as_ptr();
        glShaderSource(sh, 1, &raw const ptr, std::ptr::null());
        glCompileShader(sh);
        let mut ok = 0i32;
        glGetShaderiv(sh, GL_COMPILE_STATUS, &raw mut ok);
        if ok != 0 {
            return Ok(sh);
        }
        // `c_char` is UNSIGNED on aarch64 and signed on x86_64, so the
        // element type is named rather than written as `i8`.
        let mut log = vec![0 as c_char; 1024];
        let mut len = 0i32;
        glGetShaderInfoLog(sh, 1024, &raw mut len, log.as_mut_ptr());
        let text = String::from_utf8_lossy(
            &log[..len.max(0) as usize]
                .iter()
                .map(|c| *c as u8)
                .collect::<Vec<u8>>(),
        )
        .into_owned();
        Err(format!("a shader did not compile: {text}"))
    }
}
