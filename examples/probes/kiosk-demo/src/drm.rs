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
//! # What this does NOT do
//!
//! **No page flip, no vblank, no double buffer.** One dumb buffer, mapped once,
//! drawn into directly. Tearing is possible and does not matter: this program
//! exists to prove a capability and to show which button was pressed. A launcher
//! that animates needs more, and `§104.34` is where that is decided.
//!
//! **No lease.** `§104.34` rules that the launcher will hold master and lease
//! the display to applications, which is what makes an exit confirmation
//! possible without freezing anything. This program takes master itself, which
//! is what an application does *today* and is exactly the thing that ruling
//! changes. It is the reference for the master half, not for the lease half.

use core::ffi::{c_char, c_int, c_uint, c_void};

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

    fn open(path: *const c_char, flags: c_int, ...) -> c_int;
    fn mmap(
        addr: *mut c_void,
        len: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: i64,
    ) -> *mut c_void;
    fn __errno_location() -> *mut c_int;
}

const O_RDWR: c_int = 0o2;
const O_CLOEXEC: c_int = 0o2_000_000;
const PROT_READ: c_int = 1;
const PROT_WRITE: c_int = 2;
const MAP_SHARED: c_int = 1;
const MAP_FAILED: *mut c_void = usize::MAX as *mut c_void;

fn errno() -> i32 {
    // Safety: the C library owns this location for the calling thread.
    unsafe { *__errno_location() }
}

// ---------------------------------------------------------------------------
// The screen
// ---------------------------------------------------------------------------

/// A display this program is master of, with a buffer already scanning out.
///
/// **Nothing here is freed.** The program holds the screen until it is stopped,
/// and the kernel reclaims the mapping, the buffer and DRM master when the
/// process dies — which `ADR-015` §6.2 makes the supervisor's business, not
/// this program's. A `Drop` that dropped master would create a window in which
/// the console draws again before the process is actually gone.
pub struct Screen {
    /// The mapped scanout buffer.
    ptr: *mut u8,
    pub width: usize,
    pub height: usize,
    /// Bytes per row, which is **not** `width * 4` — the kernel picks it.
    pub stride: usize,
    len: usize,
    /// Which card, and which connector, for the record this program prints.
    pub what: String,
}

impl Screen {
    // **No per-pixel writer here, deliberately.** The buffer is `XRGB8888`, and
    // since 2026-09-04 the one place that knows it is `rivet_sdk::draw`, which
    // states the layout in its own module documentation and carries no
    // channel-offset indirection at all. ~~`draw_screen`'s `Pixel::Bytes { red:
    // 2, green: 1, blue: 0 }`~~ — `src/pixel.rs` went with the bitmap font it
    // served. A second writer on this type would be a second statement of the
    // same format, and the two would eventually disagree (ADR-011).

    /// Copy a whole rendered canvas into the scanout buffer.
    ///
    /// The canvas is built at this screen's own stride, so this is one copy and
    /// not a row loop. A shorter canvas is copied as far as it goes rather than
    /// refused: a caller that got the size wrong should see a partial screen,
    /// which is diagnosable, instead of a blank one.
    pub fn blit(&mut self, canvas: &[u8]) {
        let n = canvas.len().min(self.len);
        // Safety: `n` is within both the mapping and the slice.
        unsafe { core::ptr::copy_nonoverlapping(canvas.as_ptr(), self.ptr, n) };
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
    let fd = unsafe { open(c_path.as_ptr(), O_RDWR | O_CLOEXEC) };
    if fd < 0 {
        return Err(format!(
            "{path}: {} — this card is not there, or this process is not root. A card node is \
             never granted to an application: the display class expands to /dev/dri/renderD* \
             for a lessee, which is what every application on a Rivet is (ADR-036 §3.1). This \
             path is for a board with no launcher, run by hand (ADR-015 §6.2)",
            errno()
        ));
    }
    drive(fd, path)
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
/// # Errors
///
/// As [`take`], minus the ones about opening a node.
pub fn take_leased(fd: c_int) -> Result<Screen, String> {
    drive(fd, "a leased display")
}

/// The half of [`take`] that is the same either way.
fn drive(fd: c_int, path: &str) -> Result<Screen, String> {

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
    // **The zero case is named separately, and `ADR-042` §8.4 is why.**
    //
    // `rivet_sdk::outputs` counts the whole funnel; this probe keeps its own
    // minimal copy of the walk and takes only the half that misled a reader on
    // a board: a descriptor that lists **no** connector at all is an empty —
    // usually a revoked — lease, and has nothing to do with a cable. Reporting
    // it as one sent a session looking at an HDMI plug that had not moved.
    //
    // Not the SDK's five sentences, because this file exists to be a small
    // readable demonstration of the ioctls and duplicating a diagnosis into it
    // would be the second copy `ADR-011` warns about. The SDK is what a real
    // application uses.
    // Safety: `res` is the live resource list this function owns.
    let listed = unsafe { (*res).count_connectors };
    let (connector, mode, encoder_id) = unsafe { first_connected(fd, res) }.ok_or_else(|| {
        if listed == 0 {
            format!(
                "{path}: no connector at all is listed on this descriptor. The ioctls \
                 answered, so this is an EMPTY lease rather than a cable — a lease is \
                 emptied of its objects when the lessor revokes it (ADR-042 §8.4)"
            )
        } else {
            format!("{path}: {listed} connector(s) and no connected output with a mode — is a cable in?")
        }
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

    let (mut handle, mut pitch, mut size) = (0u32, 0u32, 0u64);
    // Safety: out-parameters are all initialised locals.
    let rc = unsafe {
        drmModeCreateDumbBuffer(fd, width, height, 32, 0, &raw mut handle, &raw mut pitch, &raw mut size)
    };
    if rc != 0 {
        return Err(format!("{path}: no dumb buffer at {width}x{height}: {rc}"));
    }

    let mut fb_id = 0u32;
    // 24-bit depth in a 32-bit pixel is `XRGB8888`, which every KMS driver
    // scans out and which `Screen::put` writes.
    // Safety: `handle` is a buffer this process just created.
    let rc = unsafe { drmModeAddFB(fd, width, height, 24, 32, pitch, handle, &raw mut fb_id) };
    if rc != 0 {
        return Err(format!("{path}: the buffer was refused as a framebuffer: {rc}"));
    }

    let mut offset = 0u64;
    // Safety: `handle` is this process's buffer.
    let rc = unsafe { drmModeMapDumbBuffer(fd, handle, &raw mut offset) };
    if rc != 0 {
        return Err(format!("{path}: the buffer could not be prepared for mapping: {rc}"));
    }

    let len = usize::try_from(size).map_err(|_| format!("{path}: buffer size {size} is absurd"))?;
    // Safety: `offset` is the kernel's own answer for this buffer on this fd.
    let ptr = unsafe {
        mmap(
            core::ptr::null_mut(),
            len,
            PROT_READ | PROT_WRITE,
            MAP_SHARED,
            fd,
            i64::try_from(offset).unwrap_or(0),
        )
    };
    if ptr == MAP_FAILED {
        return Err(format!("{path}: the buffer could not be mapped: {}", errno()));
    }

    // **Scan it out.** Until this call the buffer is memory nobody looks at.
    let mut connectors = [connector];
    let mut mode_for_call = mode;
    // Safety: one connector, count 1, and a mode the kernel handed back for it.
    let rc = unsafe {
        drmModeSetCrtc(
            fd,
            crtc_id,
            fb_id,
            0,
            0,
            connectors.as_mut_ptr(),
            1,
            &raw mut mode_for_call,
        )
    };
    if rc != 0 {
        return Err(format!("{path}: the mode could not be set: {rc}"));
    }

    // Safety: `res` is non-null and no longer read after this.
    unsafe { drmModeFreeResources(res) };

    let name = mode
        .name
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| *c as u8 as char)
        .collect::<String>();
    Ok(Screen {
        ptr: ptr.cast::<u8>(),
        width: width as usize,
        height: height as usize,
        stride: pitch as usize,
        len,
        what: format!(
            "{path} connector {connector} crtc {crtc_id} mode {name}, master {master_note}"
        ),
    })
}

/// The first connected output that has a mode, with the mode it prefers.
///
/// **The first mode, not the largest.** The kernel lists a connector's modes
/// preferred-first, so taking `modes[0]` is taking the display's own answer
/// rather than this program's opinion about resolution.
///
/// # Safety
///
/// `res` must be a live `drmModeRes` from `drmModeGetResources` on `fd`.
unsafe fn first_connected(fd: c_int, res: *mut Resources) -> Option<(u32, ModeInfo, u32)> {
    // Safety: the caller's contract.
    let count = unsafe { (*res).count_connectors };
    for i in 0..count {
        // Safety: `i` is below the kernel's own count for this array.
        let id = unsafe { *(*res).connectors.offset(i as isize) };
        // Safety: an id the kernel just listed.
        let conn = unsafe { drmModeGetConnector(fd, id) };
        if conn.is_null() {
            continue;
        }
        // Safety: non-null, and freed on every path below.
        let usable = unsafe {
            (*conn).connection == CONNECTED
                && (*conn).count_modes > 0
                && (*conn).encoder_id != 0
                && !(*conn).modes.is_null()
        };
        if usable {
            // Safety: `count_modes > 0` and `modes` is non-null.
            let mode = unsafe { *(*conn).modes };
            let encoder = unsafe { (*conn).encoder_id };
            unsafe { drmModeFreeConnector(conn) };
            return Some((id, mode, encoder));
        }
        unsafe { drmModeFreeConnector(conn) };
    }
    None
}
