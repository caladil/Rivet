//! **The screen, taken properly** — KMS through `libdrm`, not raw writes to
//! `/dev/fb0`.
//!
//! `PLAN.md` §104.33.4, ruled 2026-09-01 after the maintainer watched this
//! program lose a fight with the system console and objected that journal lines
//! kept appearing on the screen; a program that draws has to be a DRM
//! application (`.rulings/` R-352).
//!
//! # What was wrong with the framebuffer
//!
//! `cmdline.txt` ends `console=tty1`, so **`/dev/console` IS the framebuffer
//! console**. A program that opens `/dev/fb0` and writes bytes has not told the
//! kernel anything; the console keeps drawing to the same pixels, and every
//! journal line the supervisor emits lands on top of the frame this program just
//! drew. Whoever writes last wins, and the journal writes often.
//!
//! **Taking DRM master is what suspends `fbcon`.** That is the fix, and it is
//! the only thing here that is strictly necessary — the modeset and the buffer
//! are what a master then has to do to put a pixel anywhere.
//!
//! Aura never had this problem because raylib's DRM backend does all of this
//! already. This program predates the GPU binding at all (`§104.15`): it was
//! written when `/dev/fb0` was the only way to reach a screen on this board.
//!
//! # Why `libdrm` and not raw ioctls
//!
//! Ruled by the maintainer: link `libdrm` (`.rulings/` R-352). The alternative
//! was deriving `DRM_IOCTL_MODE_CREATE_DUMB`'s encoding and the UAPI structs by
//! hand, and **libdrm 2.4.131 ships `drmModeCreateDumbBuffer`,
//! `drmModeMapDumbBuffer` and `drmModeDestroyDumbBuffer`**, which removes every
//! one of those numbers from this file. `libdrm.so.2` is in `PUBLIC_LIBRARIES`
//! since `§104.26`, so the platform already promises it and the closure stays
//! empty.
//!
//! **Every declaration below was read out of the installed headers**
//! (`/br/staging/usr/include/xf86drmMode.h`, `xf86drm.h`) rather than recalled.
//! A struct whose field order is one `uint32_t` out does not fail to compile; it
//! reads the wrong memory and produces a mode nobody asked for.
//!
//! # Presenting a frame: two pages and a flip
//!
//! **The display hardware reads a block of memory from the top line to the
//! bottom, about sixty times a second, for ever.** A program with one buffer is
//! writing into the memory the scanout engine is reading, so the hardware can be
//! partway down the screen with the old image above the split and the new image
//! below it. That is tearing, and **no amount of rendering speed removes it** —
//! a faster renderer still writes into a page that is being read, it only moves
//! where the seam lands.
//!
//! So there are **two** dumb buffers. One is scanning out (the *front* page) and
//! the other is idle (the *back* page). [`Screen::blit`] copies the canvas into
//! the back page, asks the kernel to flip to it at the next vertical blank,
//! waits for the flip to complete, and then swaps the two. The scanout engine
//! never reads a page while this process writes it, so a frame is whole or it is
//! not on the glass at all.
//!
//! ## `blit` is still the only call, and that is a decision
//!
//! It would be conventional to split this into `blit` then `present`, and it is
//! deliberately **not** split. `examples/probes/kiosk-demo`,
//! `examples/templates/template-kiosk`, `cmd/rivet-launcher` and Aura's renderer
//! all draw a whole canvas and put it on the screen in one step, and none of
//! them has asked for the control that a split would buy. The first article
//! applies to the API as much as to the descriptor: an application is handed
//! what it needs, and *"copy this to the screen"* is what a kiosk application
//! needs.
//!
//! ## The block is a FEATURE. Do not remove it
//!
//! [`Screen::blit`] does not return until the flip has completed, which means it
//! returns at a vertical blank — so **a loop that draws and blits runs at the
//! display's refresh rate and cannot run faster.** That is the point.
//!
//! The maintainer's instruction, 2026-09-01: draw at 30, 40 or 60 frames a
//! second, because a kiosk application gains nothing from rendering faster
//! than a display can show (`.rulings/` R-354). Nothing throttled a kiosk loop
//! before this. `template-kiosk` sleeps a constant **after** its work, so its
//! real interval is `work + constant` and stretches silently as the work grows
//! — a frame budget nobody can see and nobody is holding to. Landing the wait
//! on the vblank gives every kiosk application correct pacing without its
//! author having to know that pacing was a thing to get right.
//!
//! A reader who takes the block for an oversight will try to make `blit`
//! asynchronous and will get both artefacts back at once: tearing, because the
//! next `blit` will overwrite a page whose flip has not landed, and an
//! unthrottled loop burning a core to produce frames no display can show.
//!
//! ## When it cannot be done, it says so
//!
//! `AGENTS.md` §3: stubs refuse, they never fake success. If the second buffer
//! cannot be allocated, cannot be registered as a framebuffer, or the flip is
//! refused by the driver, this falls back to the single-buffer behaviour
//! described below — and **says so**, in [`Screen::what`] and on stderr. A
//! silent fallback would mean tearing that nobody can explain, and an operator
//! looking at a torn screen has no way to tell it from a broken one.
//!
//! # What this does NOT do
//!
//! ~~**No page flip, no vblank, no double buffer.** One dumb buffer, mapped
//! once, drawn into directly. Tearing is possible and does not matter: this
//! program exists to prove a capability and to show which button was pressed. A
//! launcher that animates needs more, and `§104.34` is where that is decided.~~
//!
//! **Superseded 2026-09-01** by *Presenting a frame* above. The paragraph was
//! true of the probe this module was extracted from, and *"tearing does not
//! matter"* was a claim about a static menu that changes when a button is
//! pressed. It stops being true the moment an application draws a continuous
//! full-screen animation, where the seam is in every frame rather than in the
//! occasional one. It is struck rather than deleted because it records what the
//! trade-off was and why it was acceptable for as long as it was.
//!
//! **The single-buffer path still exists** and is what a fallback degrades to,
//! so the description above is also the description of the degraded mode.
//!
//! ~~**No lease.** `§104.34` rules that the launcher will hold master and lease
//! the display to applications, which is what makes an exit confirmation
//! possible without freezing anything. This program takes master itself, which
//! is what an application does *today* and is exactly the thing that ruling
//! changes. It is the reference for the master half, not for the lease half.~~
//!
//! **Superseded 2026-09-01 — and unlike the paragraph above, this one was
//! already false on the day it was written here.** It was carried across
//! verbatim from `cmd/rivet-launcher/src/drm.rs`, where every word of it is
//! still true, and was never rewritten for the crate it landed in. Two things
//! are wrong with it in this file:
//!
//! * **This is a library, not a program.** There is no "this program" to take
//!   master; there is whichever application linked the SDK.
//! * **Both halves are built here.** [`take`] is the master path, for a board
//!   with no launcher; [`take_leased`] and [`handed`] are the **lessee** path,
//!   built for `PLAN.md` §106.2, and [`handed`] is the call a kiosk application
//!   should make. `§104.34` remains where leases themselves are ruled.

// **`unsafe` is the point of this module**, as it is in `display.rs`: a
// modeset is a sequence of C calls and there is no safe spelling of one.
// Every declaration below was read out of the installed headers rather than
// recalled -- a struct one `uint32_t` out does not fail to compile, it reads
// the wrong memory and produces a mode nobody asked for.
#![allow(unsafe_code)]

use core::ffi::{c_char, c_int, c_uint, c_void};

// **The presentation vocabulary — the completion event, the timeout, and the
// page bookkeeping — is in [`crate::flip`]**, which has no C in it and can
// therefore be tested on a host with no `libdrm`. That file says why at length.
use crate::display::DisplayError;
use crate::flip::{FLIP_TIMEOUT, Flip, Rotation, flip_completed};

// ---------------------------------------------------------------------------
// The C declarations, read from the headers
// ---------------------------------------------------------------------------

/// `DRM_DISPLAY_MODE_LEN`, from `drm/drm_mode.h`.
const MODE_NAME_LEN: usize = 32;

/// `drmModeModeInfo` — `xf86drmMode.h`.
#[repr(C)]
#[derive(Clone, Copy)]
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

/// `drmModeRes`.
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

/// `drmModeConnector`. `drmModeConnection` and `drmModeSubPixel` are C enums,
/// which this toolchain lays out as `unsigned int`.
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

/// `drmModeEncoder`.
#[repr(C)]
struct Encoder {
    encoder_id: u32,
    encoder_type: u32,
    crtc_id: u32,
    possible_crtcs: u32,
    possible_clones: u32,
}

/// `DRM_MODE_CONNECTED`, from the `drmModeConnection` enum.
const CONNECTED: c_uint = 1;

// **The SDK owns the link, so an application does not.**
//
// `PLAN.md` §107.4's kiosk round is the reason. Without this every kiosk
// application needs a `build.rs` or a `.cargo/config.toml` emitting `-ldrm`,
// which is a thing to discover before anything appears on a screen — and
// discovering it is not what that round is meant to measure. `libdrm.so.2` has
// been in `PUBLIC_LIBRARIES` since `§104.26`, so this adds nothing to an
// application's declared closure: `library = []` stays true.
#[link(name = "drm")]
unsafe extern "C" {
    fn drmSetMaster(fd: c_int) -> c_int;
    fn drmModeGetResources(fd: c_int) -> *mut Resources;
    fn drmModeFreeResources(ptr: *mut Resources);
    fn drmModeGetConnector(fd: c_int, id: u32) -> *mut Connector;
    fn drmModeFreeConnector(ptr: *mut Connector);
    fn drmModeGetEncoder(fd: c_int, id: u32) -> *mut Encoder;
    fn drmModeFreeEncoder(ptr: *mut Encoder);
    fn drmModeCreateDumbBuffer(
        fd: c_int,
        width: u32,
        height: u32,
        bpp: u32,
        flags: u32,
        handle: *mut u32,
        pitch: *mut u32,
        size: *mut u64,
    ) -> c_int;
    fn drmModeMapDumbBuffer(fd: c_int, handle: u32, offset: *mut u64) -> c_int;
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
    /// `extern int drmModePageFlip(int fd, uint32_t crtc_id, uint32_t fb_id,
    /// uint32_t flags, void *user_data);` — `xf86drmMode.h` line 429.
    fn drmModePageFlip(
        fd: c_int,
        crtc_id: u32,
        fb_id: u32,
        flags: u32,
        user_data: *mut c_void,
    ) -> c_int;
    /// `extern int drmModeRmFB(int fd, uint32_t bufferId);` — line 315.
    fn drmModeRmFB(fd: c_int, buffer_id: u32) -> c_int;
    /// `extern int drmModeDestroyDumbBuffer(int fd, uint32_t handle);` — line
    /// 526. Both of these exist for **one** path: a second page that was
    /// allocated and then found unusable. Nothing else here frees anything, for
    /// the reason [`Screen`] gives.
    fn drmModeDestroyDumbBuffer(fd: c_int, handle: u32) -> c_int;
}

// `open`, `mmap`, `munmap`, errno and the O_*/PROT_*/MAP_* constants are NOT
// declared here. This crate already depends on `libc` (see this file's
// `libc::close`, `libc::munmap`, `libc::poll` and `libc::read` below) and
// `libc::open`/`libc::mmap` are the same two functions, correct for whichever
// target this crate builds for. Re-declaring them by hand would be a second
// spelling of one fact (`ADR-011`) that could silently drift from the first —
// only `libdrm`'s own symbols above have no such existing spelling to reuse.

fn errno() -> i32 {
    // Safety: the C library owns this location for the calling thread, and
    // `libc::__errno_location` is the same function this used to declare by
    // hand.
    unsafe { *libc::__errno_location() }
}

// ---------------------------------------------------------------------------
// The page flip, and the event that says it happened
// ---------------------------------------------------------------------------

/// `DRM_MODE_PAGE_FLIP_EVENT`, from `drm/drm_mode.h` line 729.
///
/// Without it the flip is scheduled and nothing is ever delivered to say it
/// landed, which would leave this module guessing at the one fact it needs.
const PAGE_FLIP_EVENT: u32 = 0x01;

// **Why the completion event is read with `read(2)` and not `drmHandleEvent`.**
//
// libdrm ships `drmHandleEvent`, and the argument at the top of this module --
// "just link libdrm" -- points at using it. It is not used, for two reasons
// specific to this call, stated here so the choice is not "corrected" later:
//
// * **`drmHandleEvent` needs `poll(2)` anyway.** It calls `read` and blocks,
//   with no timeout of its own, so the wait above it is the same code either
//   way. It saves nothing on the half that is actually hard.
// * **It would trade two `u32`s for a struct of function pointers.** Its
//   `drmEventContext` is a versioned struct of four callbacks (`xf86drm.h`
//   line 775); getting THAT layout wrong means C calling through a wrong
//   pointer. Getting `struct drm_event`'s layout wrong means reading a wrong
//   number, failing to recognise the event, and taking the timeout path this
//   module already has. **The failure modes are not comparable**, and this
//   module's own standard is that a struct one `uint32_t` out does not fail to
//   compile -- it reads the wrong memory.
//
// So the flip itself is libdrm's `drmModePageFlip`; only the drain is a read.

// ---------------------------------------------------------------------------
// The screen
// ---------------------------------------------------------------------------

/// **One page**: a dumb buffer, the framebuffer id the CRTC knows it by, and
/// the mapping this process draws through.
///
/// There are normally two of these. The handle is kept only for
/// [`discard`] — see [`Screen`] for why nothing else is ever freed.
struct Page {
    /// The kernel's handle for the dumb buffer.
    handle: u32,
    /// What `drmModeSetCrtc` and `drmModePageFlip` name this buffer by.
    fb_id: u32,
    /// The mapping.
    ptr: *mut u8,
    len: usize,
}

/// Where a [`Screen`]'s descriptor came from, which is what decides whether
/// dropping it frees anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    /// Opened by [`take`], and this process is DRM master for its whole life.
    /// **Freeing nothing is correct here** — see [`Screen`].
    Master,
    /// A lease that arrived on `RIVET_DISPLAY`. There will be others.
    Leased,
}

/// A display this program is master of, with a buffer already scanning out.
///
/// # What dropping one frees, and why the answer is not the same both ways
///
/// **On the [`take`] path — [`Origin::Master`] — nothing is freed**, and that
/// has been true since this type existed. The program holds the screen until it
/// is stopped, and the kernel reclaims the mappings, the buffers and DRM master
/// when the process dies, which `ADR-015` §6.2 makes the supervisor's business.
/// A `Drop` that dropped master would create a window in which the console
/// draws again before the process is actually gone.
///
/// **On the [`handed`] path — [`Origin::Leased`] — everything is freed**, and
/// until 2026-09-03 nothing was (`PLAN.md` §123, Wave 5.2). The argument above
/// does not carry over, because a leased `Screen` is **not** held for the life
/// of the program: `ADR-036` §8.1 obliges an application to accept a re-issued
/// descriptor after a revocation, which means building a second `Screen` and
/// letting the first go. With no `Drop` that left, every time:
///
/// * the revoked descriptor, open and useless;
/// * two full-screen dumb buffers — **16 MB at 1080p** — in CMA, which is the
///   memory a display controller has to allocate from and is not large.
///
/// A machine that has handed the screen back and forth a dozen times has lost
/// 192 MB of it, and the first symptom is a later allocation failing.
///
/// The mapping goes before the framebuffer and the buffer, so the memory is
/// never reachable through an id that no longer names it — the same order
/// [`discard`] uses, because it is the same operation.
///
/// The one thing freed on both paths, and always was, is a second page that was
/// allocated and then found unusable: it is never drawn into, and leaving a
/// whole screen's worth of scanout-capable memory allocated is a real cost for
/// no benefit.
pub struct Screen {
    /// **The card, kept so a flip is possible at all.**
    ///
    /// Before 2026-09-01 this type retained neither the descriptor nor the CRTC
    /// id, because `drmModeSetCrtc` was called once during construction and
    /// never again. `drmModePageFlip` needs both on every frame.
    fd: c_int,
    /// The CRTC being flipped, for the same reason.
    crtc_id: u32,
    /// Whether dropping this frees anything — see the type's own note.
    origin: Origin,
    /// One page or two. `pages[rotation.front]` is scanning out.
    pages: Vec<Page>,
    rotation: Rotation,
    /// Whether the missed-event note has already been printed, so a display that
    /// delivers no events writes one line rather than five a second.
    said_missed: bool,
    pub width: usize,
    pub height: usize,
    /// Bytes per row, which is **not** `width * 4` — the kernel picks it.
    pub stride: usize,
    /// Which card, which connector, and how frames reach the screen — the
    /// record this program prints.
    pub what: String,
}

impl Screen {
    // **No per-pixel writer here, deliberately.** The one place that knows the
    // byte order is `draw_screen`'s `Pixel::Bytes { red: 2, green: 1, blue: 0 }`,
    // which the font renderer already draws through. A second writer on this
    // type would be a second statement of the same format, and the two would
    // eventually disagree (ADR-011).

    /// **Whether frames reach this screen without tearing.**
    ///
    /// `false` means the second page could not be had or the driver refused a
    /// flip, and [`Screen::what`] carries the sentence saying which.
    #[must_use]
    pub const fn is_double_buffered(&self) -> bool {
        self.rotation.is_double()
    }

    /// Copy a whole rendered canvas onto the screen, and **wait for it to be
    /// on the screen**.
    ///
    /// The canvas is built at this screen's own stride, so the copy is one
    /// `memcpy` and not a row loop. A shorter canvas is copied as far as it goes
    /// rather than refused: a caller that got the size wrong should see a
    /// partial screen, which is diagnosable, instead of a blank one.
    ///
    /// # What it actually does
    ///
    /// 1. Copies into the **back** page — the one the scanout engine is not
    ///    reading.
    /// 2. Asks the kernel to flip to it at the next vertical blank.
    /// 3. **Waits** for the completion event, up to [`FLIP_TIMEOUT`].
    /// 4. Swaps: the page just shown becomes the front, the one that was on the
    ///    glass becomes the back and is what the next call draws into.
    ///
    /// # This blocks on purpose
    ///
    /// Step 3 is what paces a kiosk loop to the display's refresh rate. It is
    /// not an oversight and it is not to be made asynchronous — the module
    /// documentation says why at length, under *The block is a FEATURE*.
    ///
    /// # Two failures, handled differently
    ///
    /// **A refused flip degrades this screen to one page, permanently**, prints
    /// why, and copies the canvas again into the page that is scanning out —
    /// so the frame the caller drew is on the screen (torn, and said to be) at
    /// the end of the call either way.
    ///
    /// **A flip that was accepted but whose event never arrived swaps anyway**,
    /// and this is the one judgement call in the module. The kernel delivers an
    /// event for every accepted flip, so the overwhelmingly likely cause is that
    /// the event was missed rather than that the flip did not happen — and if
    /// the flip really is still pending, the next one is refused with `EBUSY`
    /// and the paragraph above takes over. The policy is self-correcting, and
    /// each transition prints.
    pub fn blit(&mut self, canvas: &[u8]) {
        let target = self.rotation.target();
        self.copy_into(target, canvas);
        if !self.rotation.is_double() {
            return;
        }

        let fb_id = self.pages[target].fb_id;
        match self.flip(fb_id) {
            Flip::Done => self.rotation.presented(),
            Flip::Missed(why) => {
                if !self.said_missed {
                    self.said_missed = true;
                    eprintln!(
                        "rivet-sdk: the display accepted a page flip and reported no completion: \
                         {why}. Continuing as though it landed; a flip that really is stuck will \
                         be refused next frame and this screen will say so. Not repeated."
                    );
                }
                self.rotation.presented();
            }
            Flip::Refused(why) => {
                if self.rotation.degrade(why.clone()) {
                    self.what = format!("{} — {}", self.what, self.rotation.describe());
                    eprintln!(
                        "rivet-sdk: {why}. Falling back to one buffer, so THE DISPLAY WILL TEAR \
                         from here on. This is a degraded display, not a broken one."
                    );
                }
                // The frame just drawn is sitting in a page nobody is scanning
                // out. Put it where it will be seen.
                let front = self.rotation.target();
                self.copy_into(front, canvas);
            }
        }
    }

    /// The `memcpy`, with the bound that makes it safe.
    fn copy_into(&self, page: usize, canvas: &[u8]) {
        let page = &self.pages[page];
        let n = canvas.len().min(page.len);
        // Safety: `n` is within both the mapping and the slice, and this page is
        // not the one being scanned out unless this screen has degraded to a
        // single buffer -- which is exactly what that degradation means.
        unsafe { core::ptr::copy_nonoverlapping(canvas.as_ptr(), page.ptr, n) };
    }

    /// Schedule the flip and wait for it.
    fn flip(&self, fb_id: u32) -> Flip {
        // Safety: `fd` is this screen's open card, `crtc_id` and `fb_id` are ids
        // the kernel gave this process on that card, and the user pointer is
        // null because the completion is identified by there being only one
        // flip in flight.
        let rc = unsafe {
            drmModePageFlip(self.fd, self.crtc_id, fb_id, PAGE_FLIP_EVENT, core::ptr::null_mut())
        };
        if rc != 0 {
            return Flip::Refused(format!(
                "the display refused a page flip on crtc {} to framebuffer {fb_id}: \
                 drmModePageFlip said {rc}, errno {}",
                self.crtc_id,
                errno()
            ));
        }
        match self.await_flip() {
            Ok(()) => Flip::Done,
            Err(why) => Flip::Missed(why),
        }
    }

    /// Wait for a `DRM_EVENT_FLIP_COMPLETE` on the card, or time out.
    ///
    /// The deadline is absolute rather than per-`poll`, so a signal arriving
    /// repeatedly cannot extend the wait without bound — `EINTR` resumes with
    /// what is left of the budget and not with a fresh one.
    fn await_flip(&self) -> Result<(), String> {
        let deadline = std::time::Instant::now() + FLIP_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return Err(format!("nothing arrived in {} ms", FLIP_TIMEOUT.as_millis()));
            }
            // At least 1 ms: a `poll` timeout of 0 is a non-blocking probe, and
            // spinning on one would burn the core this is meant to give back.
            let ms = c_int::try_from(left.as_millis()).unwrap_or(c_int::MAX).max(1);

            let mut fds =
                libc::pollfd { fd: self.fd, events: libc::POLLIN, revents: 0 };
            // Safety: one live `pollfd`, and a count that matches.
            let ready = unsafe { libc::poll(&raw mut fds, 1, ms) };
            if ready < 0 {
                if errno() == libc::EINTR {
                    continue;
                }
                return Err(format!("poll on the card failed: errno {}", errno()));
            }
            if ready == 0 {
                return Err(format!("nothing arrived in {} ms", FLIP_TIMEOUT.as_millis()));
            }

            // A read on a DRM descriptor returns whole events only, never a
            // partial one -- `drm.h`'s comment above `struct drm_event` says so
            // explicitly. 1 KiB holds thirty-two of the largest of them.
            let mut buf = [0u8; 1024];
            // Safety: a live buffer and its own length.
            let n = unsafe { libc::read(self.fd, buf.as_mut_ptr().cast::<c_void>(), buf.len()) };
            if n < 0 {
                if errno() == libc::EINTR {
                    continue;
                }
                return Err(format!("reading the card's events failed: errno {}", errno()));
            }
            let read = usize::try_from(n).unwrap_or(0);
            if flip_completed(&buf[..read]) {
                return Ok(());
            }
            // Something else — a vblank event from an earlier life of this
            // descriptor. Keep waiting with what is left of the budget.
        }
    }
}

/// Take the display: open the card, become master, find a connected output,
/// allocate a buffer and scan it out.
///
/// The `Err` is a sentence for a person, because every failure here is one an
/// operator can act on — a device class not granted, a cable not plugged in, a
/// driver that did not bind.
pub fn take(path: &str) -> Result<Screen, String> {
    let c_path = std::ffi::CString::new(path).map_err(|_| format!("{path} is not a path"))?;
    // Safety: a valid NUL-terminated path and flags open(2) defines.
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(format!(
            "{path}: {} — the display class was not granted, or this card is not there \
             (ADR-015 §6.2)",
            errno()
        ));
    }
    // **Closed if `drive` refuses.** The node was opened here, so a failure
    // below leaves this function owning a descriptor nobody named — which on
    // the one path that retries (a board with no cable in yet) is one leaked
    // card node per attempt.
    match drive(fd, path, Origin::Master) {
        Ok(screen) => Ok(screen),
        Err(why) => {
            // Safety: `fd` was opened above, `drive` returned an error so it
            // built no `Screen` around it, and nothing else holds it.
            unsafe { libc::close(fd) };
            Err(why)
        }
    }
}

/// **Drive a display this process was HANDED**, rather than one it opened.
///
/// `PLAN.md` §106.2. The descriptor is a **DRM lease** the launcher cut and the
/// supervisor delivered on `RIVET_DISPLAY`. Everything below `take` does is the
/// same; what changes is where the descriptor came from and that this process
/// is a **lessee** rather than master.
///
/// `drmSetMaster` is still attempted and still expected to fail — a lessee is
/// not master and cannot become one, which is the whole point of
/// the maintainer's 2026-09-01 ruling (`ADR-036` §2). It is left in place because the note it produces is the
/// clearest evidence in the log of which path a run took.
///
/// # The page flip on a leased descriptor
///
/// **A lessee owns the leased objects and may flip them** — `drmModePageFlip`
/// on a leased CRTC is the ordinary case, not a privileged one, and it is what
/// every leased compositor does. `PLAN.md` §106.1 leases the CRTC **and** its
/// connector, which is the pair a flip needs.
///
/// **This has not been run on a board** (2026-09-01). It is what the lease
/// documentation says and what the ioctl's permission check does; it is not a
/// reading off hardware, and `AGENTS.md` §10.5 is explicit that the difference
/// matters. If a lessee's flip is refused, [`Screen::blit`] degrades and says so
/// on stderr with the errno — which is the evidence that would settle it.
///
/// # It takes an `OwnedFd`, and that is the ownership being made explicit
///
/// **It took a raw `c_int` until 2026-09-03**, which left the question of who
/// closes the lease unanswered on the failing path — and unanswered meant
/// nobody. [`handed`] turned an `OwnedFd` into a raw number to call this, with
/// a comment saying the descriptor was "leaked deliberately"; that is right when
/// a `Screen` is built and wrong when one is not, and the difference was
/// invisible at the call site (`PLAN.md` §123, Wave 5.1).
///
/// Taking the `OwnedFd` says it plainly: on success the [`Screen`] owns it and
/// frees it on drop; on failure it is closed here, by going out of scope.
///
/// # Errors
///
/// As [`take`], minus the ones about opening a node.
pub fn take_leased(lease: std::os::fd::OwnedFd) -> Result<Screen, String> {
    use std::os::fd::{AsRawFd as _, IntoRawFd as _};
    let raw = lease.as_raw_fd();
    match drive(raw, "a leased display", Origin::Leased) {
        // The `Screen` owns it now and its `Drop` closes it, so this must not.
        Ok(screen) => {
            let _ = lease.into_raw_fd();
            Ok(screen)
        }
        // `lease` falls out of scope here and closes.
        Err(why) => Err(why),
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        if self.origin == Origin::Master {
            // Deliberate — see the type's own note. The kernel reclaims it all
            // when the process ends, and giving master back early would let the
            // console draw over a program that is still running.
            return;
        }
        for page in std::mem::take(&mut self.pages) {
            discard(self.fd, page);
        }
        // Safety: this descriptor arrived as an `OwnedFd` and `take_leased`
        // handed its ownership to this `Screen`. Nothing else holds it, and
        // `drop` runs once.
        unsafe {
            libc::close(self.fd);
        }
    }
}

/// Frees a `drmModeRes` however the function holding it returns.
///
/// **`drive` had six early returns after `drmModeGetResources` and freed it on
/// none of them** (`PLAN.md` §123, Wave 5.2). Each of those is a real path — no
/// connected output, no encoder, no CRTC, a page that would not allocate, a
/// modeset the driver refused — and each leaked the resource list. On the
/// `handed` path they are reached again on every re-issue.
///
/// A guard rather than six `unsafe` frees, because the seventh return added
/// later would have been the one that forgot.
struct Held(*mut Resources);

impl Drop for Held {
    fn drop(&mut self) {
        // Safety: the pointer came from `drmModeGetResources` and this is the
        // only owner; it is not read after the guard goes.
        unsafe { drmModeFreeResources(self.0) };
    }
}

/// The half of [`take`] that is the same either way.
fn drive(fd: c_int, path: &str, origin: Origin) -> Result<Screen, String> {

    // **Becoming master is the OPEN, not this call**, and getting that backwards
    // cost a deploy. `SET_MASTER` answered `EACCES` (13) and not `EBUSY`: it
    // needs `CAP_SYS_ADMIN`, and an application here runs as its own
    // unprivileged uid (`ADR-015` §5.1). The kernel makes the FIRST opener of a
    // node that has no current master into master by itself, so by the time this
    // runs the job is already done and the ioctl is asking for a privilege
    // nothing needs.
    //
    // It is still attempted, because it IS the call that matters on the path
    // where a process had master, dropped it, and wants it back — which is what
    // `§104.34`'s launcher will do around a lease. A failure is reported and not
    // fatal: `drmModeSetCrtc` below is the authority on whether this process can
    // actually drive the display, and it refuses if it cannot.
    //
    // Safety: `fd` is an open DRM node.
    let master_note = if unsafe { drmSetMaster(fd) } == 0 {
        "explicitly".to_string()
    } else {
        format!("implicitly, by opening it (SET_MASTER said {})", errno())
    };

    // Safety: `fd` is an open DRM node; the pointer is checked before use and
    // the count/array pairs come from the kernel together.
    let res = unsafe { drmModeGetResources(fd) };
    if res.is_null() {
        return Err(format!("{path}: no KMS resources — is this a modesetting driver?"));
    }
    // From here every return frees it, including the ones below that do not
    // mention it. See `Held`.
    let held = Held(res);
    // **Four counts, not a boolean** — `crate::outputs`. The step at which the
    // funnel closes is the diagnosis, and reporting every one of them as
    // *"is a cable in?"* named the wrong thing on a board whose cable had not
    // moved (`ADR-042` §8.4).
    let (survey, found) = unsafe { survey_connectors(fd, res) };
    let (connector, mode, encoder_id) = found.ok_or_else(|| {
        let held = match origin {
            Origin::Master => crate::outputs::Held::Opened,
            Origin::Leased => crate::outputs::Held::Leased,
        };
        format!("{path}: {}", survey.why(held))
    })?;

    // The CRTC to drive it with, taken from the connector's current encoder.
    // Safety: `encoder_id` came from a connector the kernel just described.
    let encoder = unsafe { drmModeGetEncoder(fd, encoder_id) };
    if encoder.is_null() {
        return Err(format!("{path}: the connected output names no encoder"));
    }
    // Safety: non-null, and freed below.
    let crtc_id = unsafe { (*encoder).crtc_id };
    unsafe { drmModeFreeEncoder(encoder) };
    if crtc_id == 0 {
        return Err(format!(
            "{path}: the output's encoder is bound to no CRTC, so there is nothing to scan out \
             with. A full modeset would pick one; this program does not."
        ));
    }

    let width = u32::from(mode.hdisplay);
    let height = u32::from(mode.vdisplay);

    // **The front page.** Its failure is fatal: without it there is nothing to
    // scan out and nothing this function could return.
    let (front, pitch) = allocate_page(fd, width, height).map_err(|why| format!("{path}: {why}"))?;

    // **Scan it out.** Until this call the buffer is memory nobody looks at.
    let mut connectors = [connector];
    let mut mode_for_call = mode;
    // Safety: one connector, count 1, and a mode the kernel handed back for it.
    let rc = unsafe {
        drmModeSetCrtc(
            fd,
            crtc_id,
            front.fb_id,
            0,
            0,
            connectors.as_mut_ptr(),
            1,
            &raw mut mode_for_call,
        )
    };
    if rc != 0 {
        // **The front page goes back**, which it did not before: this is the
        // last thing that can fail after a whole screen of scanout-capable
        // memory has been allocated, and on the `handed` path it is reached
        // again on every re-issue.
        discard(fd, front);
        return Err(format!("{path}: the mode could not be set: {rc}"));
    }

    // The resource list is not read again, so it goes now rather than at the
    // end of the function. Everything after this point is about the pages.
    drop(held);

    // **The back page**, and its failure is NOT fatal. A screen that tears is a
    // worse screen than one that does not; a screen that refuses to exist is no
    // screen at all, and an application that drew nothing is harder to diagnose
    // than one that drew something with a seam in it. So this degrades, and says
    // so — `AGENTS.md` §3's rule, in the direction where the honest answer is
    // "worse, and here is why".
    //
    // **After the modeset, not before**, so that a `drive` which fails above has
    // not allocated a whole screen of memory it is about to walk away from.
    let second = allocate_page(fd, width, height);

    // `drmModeSetCrtc` above was given `front`, so `front` is index 0 and that
    // is what `Rotation::double` assumes.
    let (pages, rotation) = match second {
        // **The two pages must be identical in shape.** A canvas is built at
        // one stride and copied into whichever page is idle; two strides would
        // put every other frame on the screen skewed, which looks like a
        // rendering bug and is not one. The kernel does not vary this for two
        // identical requests, and checking costs one comparison.
        Ok((back, back_pitch)) if back_pitch == pitch && back.len == front.len => {
            (vec![front, back], Rotation::double())
        }
        Ok((back, back_pitch)) => {
            let why = format!(
                "the second buffer came back {} bytes per row against the first's {pitch} \
                 ({} bytes against {}), and two pages of different shapes cannot hold the \
                 same canvas",
                back_pitch, back.len, front.len
            );
            discard(fd, back);
            (vec![front], Rotation::single(why))
        }
        Err(why) => (vec![front], Rotation::single(why)),
    };

    if !rotation.is_double() {
        eprintln!(
            "rivet-sdk: {path}: {}. Every frame is drawn into the memory the display is \
             reading, so the picture WILL TEAR. This is a degraded display, not a broken one.",
            rotation.single_because.as_deref().unwrap_or("no second page")
        );
    }

    let name = mode
        .name
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| *c as u8 as char)
        .collect::<String>();
    Ok(Screen {
        fd,
        crtc_id,
        origin,
        what: format!(
            "{path} connector {connector} crtc {crtc_id} mode {name}, master {master_note}, {}",
            rotation.describe()
        ),
        pages,
        rotation,
        said_missed: false,
        width: width as usize,
        height: height as usize,
        stride: pitch as usize,
    })
}

/// **Allocate one page**: a dumb buffer, registered as a framebuffer, mapped.
///
/// Returns the page and its pitch, which the caller compares between pages.
///
/// # Errors
///
/// A sentence naming the step that failed. Every one of them is a step
/// [`drive`] used to perform inline for the single buffer it had; the only
/// change is that they are now performed twice and the second set is allowed to
/// fail.
fn allocate_page(fd: c_int, width: u32, height: u32) -> Result<(Page, u32), String> {
    let (mut handle, mut pitch, mut size) = (0u32, 0u32, 0u64);
    // Safety: out-parameters are all initialised locals.
    let rc = unsafe {
        drmModeCreateDumbBuffer(
            fd,
            width,
            height,
            32,
            0,
            &raw mut handle,
            &raw mut pitch,
            &raw mut size,
        )
    };
    if rc != 0 {
        return Err(format!("no dumb buffer at {width}x{height}: {rc}"));
    }

    let mut fb_id = 0u32;
    // 24-bit depth in a 32-bit pixel is `XRGB8888`, which every KMS driver
    // scans out and which the font renderer writes.
    // Safety: `handle` is a buffer this process just created.
    let rc = unsafe { drmModeAddFB(fd, width, height, 24, 32, pitch, handle, &raw mut fb_id) };
    if rc != 0 {
        // Safety: `handle` is this process's buffer and nothing else refers to
        // it -- the framebuffer registration is what just failed.
        unsafe { drmModeDestroyDumbBuffer(fd, handle) };
        return Err(format!("the buffer was refused as a framebuffer: {rc}"));
    }

    let mut offset = 0u64;
    // Safety: `handle` is this process's buffer.
    let rc = unsafe { drmModeMapDumbBuffer(fd, handle, &raw mut offset) };
    if rc != 0 {
        // Safety: as above, plus a framebuffer id this process just registered.
        unsafe {
            drmModeRmFB(fd, fb_id);
            drmModeDestroyDumbBuffer(fd, handle);
        }
        return Err(format!("the buffer could not be prepared for mapping: {rc}"));
    }

    let len = usize::try_from(size).map_err(|_| format!("buffer size {size} is absurd"))?;
    // Safety: `offset` is the kernel's own answer for this buffer on this fd.
    let ptr = unsafe {
        libc::mmap(
            core::ptr::null_mut(),
            len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            i64::try_from(offset).unwrap_or(0),
        )
    };
    if ptr == libc::MAP_FAILED {
        // Safety: as above.
        unsafe {
            drmModeRmFB(fd, fb_id);
            drmModeDestroyDumbBuffer(fd, handle);
        }
        return Err(format!("the buffer could not be mapped: {}", errno()));
    }

    Ok((Page { handle, fb_id, ptr: ptr.cast::<u8>(), len }, pitch))
}

/// **Give a page back**, for the one path that has to: a second page that was
/// allocated and then found unusable.
///
/// The mapping goes before the framebuffer, so there is no moment where the
/// buffer is reachable through a framebuffer id that no longer names it.
fn discard(fd: c_int, page: Page) {
    // Safety: `page` is owned here and nothing else refers to it -- it was
    // never handed to a `Screen`.
    unsafe {
        libc::munmap(page.ptr.cast::<c_void>(), page.len);
        drmModeRmFB(fd, page.fb_id);
        drmModeDestroyDumbBuffer(fd, page.handle);
    }
}

/// The first connected output that has a mode, **and what was seen looking for
/// it**.
///
/// **The first mode, not the largest.** The kernel lists a connector's modes
/// preferred-first, so taking `modes[0]` is taking the display's own answer
/// rather than this program's opinion about resolution.
///
/// # Why it counts as well as searching
///
/// ~~`first_connected` returned `Option` alone~~ — and every way of failing
/// produced the one sentence *"no connected output with a mode — is a cable
/// in?"*. A descriptor listing **zero** connectors, which is what a revoked
/// lease is, read as a cable that had been unplugged. `crate::outputs` is the
/// argument in full; the counts are gathered here because this is the only pass
/// over the connectors and a second one would be a second answer.
///
/// The walk still stops at the first usable output, so the later counts are
/// *"of the ones examined"* rather than totals. That is deliberate and costs
/// nothing: they are only read when nothing usable was found, in which case
/// every connector was examined.
///
/// # Safety
///
/// `res` must be a live `drmModeRes` from `drmModeGetResources` on `fd`.
unsafe fn survey_connectors(
    fd: c_int,
    res: *mut Resources,
) -> (crate::outputs::Survey, Option<(u32, ModeInfo, u32)>) {
    let mut seen = crate::outputs::Survey::default();
    // Safety: the caller's contract.
    let count = unsafe { (*res).count_connectors };
    seen.listed = usize::try_from(count).unwrap_or(0);
    for i in 0..count {
        // Safety: `i` is below the kernel's own count for this array.
        let id = unsafe { *(*res).connectors.offset(i as isize) };
        // Safety: an id the kernel just listed.
        let conn = unsafe { drmModeGetConnector(fd, id) };
        if conn.is_null() {
            continue;
        }
        seen.described += 1;
        // Safety: non-null, and freed on every path below.
        let (connected, has_mode, has_encoder) = unsafe {
            (
                (*conn).connection == CONNECTED,
                (*conn).count_modes > 0 && !(*conn).modes.is_null(),
                (*conn).encoder_id != 0,
            )
        };
        if connected {
            seen.connected += 1;
            if has_mode {
                seen.with_mode += 1;
                if has_encoder {
                    seen.with_encoder += 1;
                    // Safety: `count_modes > 0` and `modes` is non-null.
                    let mode = unsafe { *(*conn).modes };
                    let encoder = unsafe { (*conn).encoder_id };
                    unsafe { drmModeFreeConnector(conn) };
                    return (seen, Some((id, mode, encoder)));
                }
            }
        }
        unsafe { drmModeFreeConnector(conn) };
    }
    (seen, None)
}

/// **The screen a Rivet handed this application** — `PLAN.md` §106.2.
///
/// The one call a kiosk application should need. It waits on the channel named
/// by `RIVET_DISPLAY`, takes the display lease the launcher cut for it, and
/// returns something that can be drawn on.
///
/// # Why this and not [`take`]
///
/// [`take`] opens a device node and asks to be DRM master, which is right on a
/// board with no launcher and wrong everywhere else: on a machine that has one,
/// the launcher is master and stays master, and an application that grabbed it
/// would take the screen from the program that arbitrates it. **An application
/// is HANDED a display; it does not go and take one** (`ADR-036` §2).
///
/// # It returns the CHANNEL as well, and that is the whole point
///
/// **This returned a bare `Screen` until 2026-09-03, and the `Channel` was
/// dropped at the end of this function** — `PLAN.md` §123, Wave 5.1.
/// `crate::display`'s own documentation says of exactly that shape: *"an
/// application that read one and dropped this would work until the first
/// re-issue and then never draw again."* It was the headline constructor of the
/// SDK and it made the obligation `ADR-036` §8.1 attaches to
/// `devices = ["display"]` **impossible to honour**, because the socket the
/// second descriptor arrives on was already closed.
///
/// So it hands back both, and the application keeps the channel:
///
/// ```no_run
/// let mut handed = rivet_sdk::screen::handed()?;
/// let canvas = vec![0u8; handed.screen.stride * handed.screen.height];
/// loop {
///     handed.screen.blit(&canvas);
///     // Once a frame, without blocking. `ADR-036` §8.1's obligation, in
///     // three lines.
///     if let Some(lease) = handed.channel.try_next()? {
///         handed.screen = rivet_sdk::screen::take_leased(lease)?;
///     }
/// }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// Assigning over `handed.screen` drops the old one, which now frees its
/// descriptor and its buffers — see [`Screen`].
///
/// # Errors
///
/// A typed [`DisplayError`], because the two ends of that type mean opposite
/// things to a caller and a `String` made them one thing:
/// [`DisplayError::NotGranted`] is *carry on, you are not on the screen*, and
/// [`DisplayError::Closed`] is *exit*. **`Err` is an ordinary state and not a
/// crash**: an application started on a machine with no launcher has no
/// channel, and the right response is to say so and go on doing whatever else
/// it does.
pub fn handed() -> Result<Handed, DisplayError> {
    let channel = crate::display::Channel::from_env()?;
    let lease = channel.next()?;
    let screen = take_leased(lease).map_err(DisplayError::NotDriveable)?;
    Ok(Handed { channel, screen })
}

/// What [`handed`] gives an application: a screen, and the channel the **next**
/// screen will arrive on.
///
/// Both, always, because an application that has one and not the other cannot
/// meet `ADR-036` §8.1 — see [`handed`] for the worked loop.
pub struct Handed {
    /// Where a re-issued display arrives. Poll it with
    /// [`crate::display::Channel::try_next`] once a frame.
    pub channel: crate::display::Channel,
    /// The screen to draw on now.
    pub screen: Screen,
}

impl Screen {
    /// **Take a picture of what this application drew** — `PLAN.md` §108.5.
    ///
    /// Writes `screen.qoi` into the directory the supervisor handed over in
    /// `RIVET_APP_DATA`, which an operator can then fetch with
    /// `rivet get file`.
    ///
    /// # Why an application has to do this itself
    ///
    /// Because nothing else can. While this application holds the display
    /// lease it owns the CRTC, and the launcher — still DRM master — is not
    /// drawing and has no view of these pixels. Asked for a capture in that
    /// state the launcher **refuses**, and it is right to: the frame it still
    /// holds is the one from before it handed the screen over, and returning
    /// that would be a real screenshot of the wrong moment.
    ///
    /// So the frame belongs to whoever drew it. **This is what makes a kiosk
    /// application's work visible to somebody who is not standing in front of
    /// it** — an operator, or an agent that cannot stand anywhere at all
    /// (`ADR-012`).
    ///
    /// `canvas` is the buffer just passed to [`Screen::blit`]. Taking it as an
    /// argument rather than reading the mapped buffer back is deliberate: the
    /// scanout engine is walking that memory, and what was DRAWN is a better
    /// answer than what is being displayed halfway through a frame.
    ///
    /// # Errors
    ///
    /// A sentence for a journal: an empty canvas, no `RIVET_APP_DATA`, or the
    /// write failing.
    pub fn capture(&self, canvas: &[u8]) -> Result<String, String> {
        if canvas.is_empty() {
            return Err("nothing has been drawn yet".to_string());
        }
        let dir = crate::app::data().map_err(|e| e.to_string())?;
        let path = dir.join("screen.qoi");
        // `None` is a canvas that does not hold the screen it is being asked to
        // describe. Reported rather than encoded short — see `qoi::encode`.
        let encoded = crate::qoi::encode(canvas, self.width, self.height, self.stride)
            .ok_or_else(|| {
                format!(
                    "the canvas is {} bytes and this screen is {}x{} at a stride of {}, which \
                     needs {}. Nothing was written: a QOI whose header says more than its body \
                     carries is a screenshot that decodes into a sheared image with no error",
                    canvas.len(),
                    self.width,
                    self.height,
                    self.stride,
                    self.height.saturating_sub(1) * self.stride + self.width * 4,
                )
            })?;
        // Written to a neighbour and renamed, so a reader never sees half a
        // file. `rename(2)` within one directory is atomic; a write in place is
        // not, and the reader is somebody over a control plane with no way to
        // know they caught it mid-write.
        let temp = path.with_extension("qoi.new");
        std::fs::write(&temp, &encoded).map_err(|e| format!("{}: {e}", temp.display()))?;
        std::fs::rename(&temp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(format!("{} ({} bytes)", path.display(), encoded.len()))
    }
}
