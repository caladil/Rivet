//! **The boot splash** — the shield, the name, and the version, drawn once on
//! the framebuffer.
//!
//! # What this is and what it deliberately is not
//!
//! It is a **one-shot blit**. It opens `/dev/fb0`, draws, and exits. There is no
//! loop, no animation and no spinner — the maintainer ruled the spinner out of
//! scope on 2026-08-30, and the reason it is worth keeping out is stronger than
//! scope: *a spinner cannot tell "working" from "wedged"*. It turns at the same
//! rate either way, which is the one thing a person watching a slow boot wants
//! to know.
//!
//! # Why a separate program rather than code in pid 1
//!
//! `ADR-035` §8A.1.2 constrains pid 1 to what must be *small, must not allocate
//! unboundedly, and must not block* — the same rule that made break-glass a line
//! reader instead of a server. A megabyte of artwork and a scaler are the
//! opposite of that, so this is a child pid 1 execs and reaps like any other.
//! If it crashes, the boot does not.
//!
//! # Why a framebuffer and not a graphics stack
//!
//! Because there is not one. `/dev/dri` is **absent** on this board — no DRM, no
//! Mesa, no compositor (`docs/design-candidates/Shared-Libraries-and-the-Build-Target.md`
//! §5.7.1 is the survey). `/dev/fb0` is what the Pi firmware leaves behind, and
//! a blit into it needs nothing that is not already in the image.
//!
//! # It never becomes the reason a boot failed
//!
//! Every failure here is reported on stderr and exits **zero**. A missing
//! framebuffer, an unreadable geometry, a short mapping — none of them are worth
//! a boot. The screen is decoration; the journal and the control plane are the
//! machine's account of itself.

use std::fmt::Write as _;
use std::fs;
use std::io::{Seek, SeekFrom, Write};

mod endpoint;
mod font;
mod pixel;

use crate::pixel::{Channel, Pixel};

/// What the splash was told to draw.
///
/// **Every one of these is a DOCUMENT decision that pid 1 translates.** The
/// splash holds no policy: `ADR-011` puts the configuration in one place, and
/// a program that read the document itself would be a second reader of it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Options {
    /// Draw the endpoint line at all. The maintainer asked for the option on
    /// security grounds (.rulings/ R-343) — a screen in a public place naming the
    /// admin port is a thing somebody may not want.
    endpoint: bool,
    /// Draw the version line. Same argument: a version tells a visitor which
    /// vulnerabilities to look up.
    version: bool,
    /// The control-plane port, from `[control-plane] listen`.
    port: u16,
    /// A name to show instead of the address — `devpi.local`.
    ///
    /// **Decided by pid 1, never here.** It is set only when the document names
    /// a hostname AND `rivet-mdnsd` is enabled, because either alone gives a
    /// name nothing resolves. `rivet_config::splash::Splash::endpoint_name`
    /// holds that reasoning and this program just draws what it is handed.
    name: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        // Both lines ON by default, which is the maintainer's ruling: the
        // screen is most useful when it says where to reach the machine, and
        // the suppression is for the case where that is not wanted.
        Self {
            endpoint: true,
            version: true,
            port: DEFAULT_PORT,
            name: None,
        }
    }
}

/// `ADR-035`'s port, used when nothing said otherwise.
const DEFAULT_PORT: u16 = 7777;

/// How long to keep looking for an address after the first draw.
///
/// **The splash is drawn BEFORE the network exists**, and on a DHCP boot the
/// address arrives ten to forty seconds later — measured on the board on
/// 2026-08-30, where the lease took until the carrier came up. So the first
/// draw carries no endpoint and this program stays alive to add one.
///
/// Bounded, because a splash that waited forever would be a daemon, and this
/// program's whole claim is that it is not one.
const WATCH_ATTEMPTS: u32 = 90;

/// How long between those looks.
const WATCH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// Read the flags pid 1 passes.
///
/// Unknown flags are REPORTED AND IGNORED rather than fatal: this program's
/// failure mode must never be a boot without a picture, and an older pid 1
/// starting a newer splash is a combination an A/B update can produce.
fn options(args: &[String]) -> Options {
    let mut out = Options::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--no-endpoint" => out.endpoint = false,
            "--no-version" => out.version = false,
            "--name" => {
                i += 1;
                match args.get(i) {
                    Some(name) if !name.trim().is_empty() => {
                        out.name = Some(name.trim().to_string());
                    }
                    _ => report("--name needs a name; the address will be shown instead"),
                }
            }
            "--port" => {
                i += 1;
                match args.get(i).and_then(|p| p.parse::<u16>().ok()) {
                    Some(port) => out.port = port,
                    None => report(&format!(
                        "--port needs a number and got {:?}; using {DEFAULT_PORT}",
                        args.get(i)
                    )),
                }
            }
            other => report(&format!("ignoring unknown argument {other:?}")),
        }
        i += 1;
    }
    out
}

/// The shield, as raw RGBA at a fixed size.
///
/// **Regenerate with**, from the repository root:
///
/// ```text
/// ffmpeg -i <artwork>.png -vf "scale=512:512:flags=lanczos" \
///        -pix_fmt rgba -f rawvideo cmd/rivet-splash/assets/shield-512.rgba -y
/// ```
///
/// Raw rather than a PNG because decoding one would mean a decoder in the
/// image for a picture that never changes between boots. The cost is a
/// megabyte on a 2 GiB card and the saving is a dependency.
const SHIELD: &[u8] = include_bytes!("../../../assets/shield-512.rgba");

/// The shield's edge in the asset above.
const SHIELD_SIZE: usize = 512;

/// Where the framebuffer's geometry is published.
const FB_SYS: &str = "/sys/class/graphics/fb0";

/// The framebuffer device.
const FB_DEV: &str = "/dev/fb0";

/// **The background.** Near-black rather than black, so a live splash is
/// distinguishable from a dead display.
const BACKGROUND: [u8; 3] = [0x0d, 0x11, 0x17];

/// The text under the shield.
const INK: [u8; 3] = [0xe8, 0xec, 0xf0];

/// The version line, dimmer than the name — it is reference, not identity.
const INK_DIM: [u8; 3] = [0x8a, 0x97, 0xa6];

/// Say something, where it can still be read.
///
/// **`/dev/kmsg` and not just stderr**, since 2026-08-30: pid 1 now points its
/// own stdout and stderr at `/dev/null` before starting this program, because
/// the supervisor inherits them and its journal echo was painting over the
/// picture (`cmd/rivet-initd/src/splash.rs`). That makes stderr alone a place
/// nobody will ever look. The ring buffer is read back by the supervisor
/// (`ADR-022` §4.1), so a splash that could not draw says so in `get log`.
///
/// Both destinations are best effort and every failure is dropped — this
/// function exists to report a decoration failing, and it must never become one
/// itself.
fn report(what: &str) {
    let line = format!("rivet-splash: {what}");
    eprintln!("{line}");
    if let Ok(mut kmsg) = fs::File::options().write(true).open("/dev/kmsg") {
        let _ = kmsg.write_all(line.as_bytes());
    }
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(why) = draw(options(&args)) {
        // Reported and survivable. See the module header: a splash is never
        // the reason a boot failed.
        report(&format!("not drawn: {why}"));
    }
    std::process::ExitCode::SUCCESS
}

/// The screen, as the framebuffer describes itself.
struct Screen {
    width: usize,
    height: usize,
    stride: usize,
    bytes_per_pixel: usize,
    /// How one pixel is written.
    ///
    /// **Read from the driver, never assumed.** The first draft hard-coded
    /// `BGRX` on the reasoning that the Pi's 32-bit mode is little-endian
    /// XRGB — which is probably true of that board and is a guess about every
    /// other one. `FBIOGET_VSCREENINFO` is the driver stating it, and a splash
    /// that renders a grey shield blue on somebody else's panel is a bug that
    /// only a screen can find.
    ///
    /// **And the depth is the driver's too.** VC4's KMS framebuffer is 16 bits
    /// per pixel — `drm_fbdev_dma_setup(drm, 16)`, a constant in the driver —
    /// so this stopped being a byte offset and became `crate::pixel::Pixel` on
    /// 2026-08-31. `PLAN.md` §104.13.
    pixel: Pixel,
}

/// `fb_var_screeninfo`, as far as the colour bitfields — the rest of the struct
/// is not read and is present so the offsets are right.
#[repr(C)]
#[derive(Default)]
struct FbVarScreenInfo {
    xres: u32,
    yres: u32,
    xres_virtual: u32,
    yres_virtual: u32,
    xoffset: u32,
    yoffset: u32,
    bits_per_pixel: u32,
    grayscale: u32,
    red: FbBitfield,
    green: FbBitfield,
    blue: FbBitfield,
    transp: FbBitfield,
    // Everything past here is padding as far as this program is concerned.
    rest: [u32; 26],
}

/// One channel's position within a pixel.
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct FbBitfield {
    offset: u32,
    length: u32,
    msb_right: u32,
}

/// `FBIOGET_VSCREENINFO`.
///
/// **Untyped, and cast at the call site with `as _`**, because `libc::ioctl`'s
/// request argument is not the same type everywhere: it is `c_int` on
/// aarch64-musl and `c_ulong` on the glibc host this was first compiled for. A
/// constant typed for one of them fails to build for the other, which is how
/// this was found — the host check passed and the card build did not.
const FBIOGET_VSCREENINFO: u32 = 0x4600;

/// Ask the driver how a pixel is laid out.
///
/// Returns `None` when the ioctl is refused, and the caller then falls back to
/// the common little-endian `BGRX` — **and says so**, because a fallback that
/// looked like a reading is how a wrong colour becomes nobody's fault.
///
/// `Some(Err(..))` is different from `None` and the difference matters: the
/// driver answered and described something this program cannot draw. That is a
/// refusal with the numbers in it, not a fallback.
fn pixel_format(fd: std::os::fd::RawFd) -> Option<Result<Pixel, String>> {
    let mut info = FbVarScreenInfo::default();
    // SAFETY: `info` is a live, correctly-aligned `FbVarScreenInfo` owned by
    // this frame, and `FBIOGET_VSCREENINFO` writes exactly that struct into
    // it. `fd` is a descriptor this program opened and still holds. The
    // return code is checked before anything is read back.
    let rc = unsafe {
        libc::ioctl(
            fd,
            FBIOGET_VSCREENINFO as _,
            std::ptr::addr_of_mut!(info).cast::<libc::c_void>(),
        )
    };
    if rc != 0 {
        return None;
    }
    let ch = |b: FbBitfield| Channel {
        offset: b.offset,
        length: b.length,
    };
    Some(Pixel::of(
        info.bits_per_pixel,
        ch(info.red),
        ch(info.green),
        ch(info.blue),
    ))
}

/// Read the geometry from `sysfs` rather than assuming it.
///
/// **`virtual_size` and not a guess**, because the Pi's framebuffer is whatever
/// the display negotiated: the board this was written against reports
/// `1824,984` — 1920×1080 with overscan taken off — and a program that assumed
/// 1920 would draw off the edge of every line.
fn geometry() -> Result<Screen, String> {
    let read = |name: &str| -> Result<String, String> {
        fs::read_to_string(format!("{FB_SYS}/{name}"))
            .map(|s| s.trim().to_string())
            .map_err(|e| format!("{FB_SYS}/{name}: {e}"))
    };

    let size = read("virtual_size")?;
    let (w, h) = size
        .split_once(',')
        .ok_or_else(|| format!("virtual_size is {size:?}, which is not `width,height`"))?;
    let width: usize = w.parse().map_err(|_| format!("width {w:?}"))?;
    let height: usize = h.parse().map_err(|_| format!("height {h:?}"))?;

    let bpp: usize = read("bits_per_pixel")?
        .parse()
        .map_err(|_| "bits_per_pixel is not a number".to_string())?;
    // **The depth is not checked here any more, and that is the point.**
    // It used to refuse anything but 32, which was right about the danger —
    // drawing 32-bit pixels into a 16-bit framebuffer is colourful noise — and
    // wrong about the remedy, which is to draw the format the driver has rather
    // than to insist on one. `crate::pixel::Pixel::of` decides from the
    // driver's own bitfields and refuses by name what it cannot write; a depth
    // alone cannot decide it, because RGB565 and RGB555 are both sixteen.
    // `PLAN.md` §104.13.

    // `stride` is the bytes per LINE and is not always `width * 4`: a driver may
    // pad. Reading it is the difference between a picture and a diagonal smear.
    let stride: usize = read("stride")?
        .parse()
        .map_err(|_| "stride is not a number".to_string())?;

    Ok(Screen {
        width,
        height,
        stride,
        bytes_per_pixel: bpp / 8,
        // Filled by the caller, which has the descriptor.
        pixel: Pixel::Bytes {
            red: 2,
            green: 1,
            blue: 0,
        },
    })
}

fn draw(options: Options) -> Result<(), String> {
    let mut fb = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(FB_DEV)
        .map_err(|e| format!("{FB_DEV}: {e}"))?;

    let mut screen = geometry()?;
    match pixel_format(std::os::fd::AsRawFd::as_raw_fd(&fb)) {
        // The driver answered and named a layout this program cannot write.
        // A refusal, not a fallback — `AGENTS.md` §3.
        Some(Err(why)) => return Err(why),
        Some(Ok(pixel)) => screen.pixel = pixel,
        None => report(
            "FBIOGET_VSCREENINFO was refused, so the channel order is ASSUMED to be \
             little-endian BGRX. If the shield looks blue, this is why",
        ),
    }

    // **Drawn immediately, with whatever is known now.** On a DHCP boot that is
    // no endpoint at all. The alternative — waiting until there is one — would
    // leave the console on screen for the whole of DHCP, which is the opposite
    // of what a splash is for.
    let mut shown = options.endpoint.then(endpoint::current_address).flatten();
    paint(&mut fb, &screen, &render(&screen, shown, &options))?;

    // Nothing to wait for if the line was suppressed, or if the address was
    // already there — which is what an ordinary static-address boot looks like.
    if !options.endpoint || shown.is_some() {
        return Ok(());
    }

    // **Then wait for the address to land and add the line**, which is the
    // maintainer's requirement of 2026-08-30 given the ordering above.
    for _ in 0..WATCH_ATTEMPTS {
        std::thread::sleep(WATCH_INTERVAL);
        let now = endpoint::current_address();
        if now == shown {
            continue;
        }
        shown = now;
        paint(&mut fb, &screen, &render(&screen, shown, &options))?;
        if shown.is_some() {
            // The line is on the screen; there is nothing left for this
            // process to do, and staying alive to watch for a change would
            // make it the daemon it claims not to be.
            return Ok(());
        }
    }
    report("no address appeared, so the splash carries no endpoint line");
    Ok(())
}

/// Put a rendered canvas on the framebuffer.
fn paint(fb: &mut fs::File, screen: &Screen, canvas: &[u8]) -> Result<(), String> {
    let _ = screen;
    fb.seek(SeekFrom::Start(0))
        .map_err(|e| format!("{FB_DEV}: {e}"))?;
    fb.write_all(canvas)
        .map_err(|e| format!("{FB_DEV}: {e}"))?;
    Ok(())
}

/// Compose the whole screen into a buffer.
///
/// **Separated from the device write so it can be LOOKED AT.** The layout,
/// the scaling, the alpha and the font are all things a test can check on a
/// machine with no framebuffer; only the write itself needs a board. Before
/// this split the entire program was unverifiable without a boot.
fn render(screen: &Screen, address: Option<std::net::Ipv4Addr>, options: &Options) -> Vec<u8> {
    // One buffer for the whole screen, filled here and written once. A
    // line-by-line write would show the drawing happening, which looks like a
    // fault rather than a splash.
    let mut canvas = vec![0u8; screen.stride * screen.height];
    fill(&mut canvas, screen, BACKGROUND);

    // **A quarter of the screen's height**, which is the maintainer's spec, and
    // horizontally centred.
    let target = screen.height / 4;
    let shield_x = screen.width.saturating_sub(target) / 2;
    // Placed so the shield and the two text lines sit as one block, centred
    // vertically with a little lift — text below an image reads better slightly
    // above centre than dead on it.
    // The block's height depends on how many lines there are, so suppressing
    // one re-centres the rest rather than leaving a gap where it was.
    let lines = 1 + usize::from(options.version) + usize::from(address.is_some());
    let block = target + font::LINE_HEIGHT * (lines + 1);
    let shield_y = screen.height.saturating_sub(block) / 2;

    blit_shield(&mut canvas, screen, shield_x, shield_y, target);

    let name = "Rivet OS";
    let version = version_line();
    let name_scale = 3;
    let version_scale = 2;

    let name_w = font::text_width(name, name_scale);
    let name_y = shield_y + target + font::LINE_HEIGHT;
    font::draw(
        &mut canvas,
        &screen_ref(screen),
        screen.width.saturating_sub(name_w) / 2,
        name_y,
        name,
        name_scale,
        INK,
    );

    let mut y = name_y + font::LINE_HEIGHT * name_scale;
    if options.version {
        let ver_w = font::text_width(&version, version_scale);
        font::draw(
            &mut canvas,
            &screen_ref(screen),
            screen.width.saturating_sub(ver_w) / 2,
            y,
            &version,
            version_scale,
            INK_DIM,
        );
        y += font::LINE_HEIGHT * version_scale;
    }

    // **Line three, and only once there is an address to put in it.** An empty
    // slot would be worse than no line: a screen that says nothing where it
    // normally says something reads as a fault.
    if let Some(address) = address {
        // **The name replaces the address; it does not arrive sooner.** The
        // maintainer's correction, 2026-08-30: the line should still not appear
        // until it is active (.rulings/ R-343). A name is known the moment this
        // program starts, and drawing it then would put an endpoint on the screen
        // before the machine could answer at it — which is a screen that lies
        // for thirty seconds every boot. The address is the PROOF the network
        // is up; the name is only a nicer way to say where.
        let text = match &options.name {
            Some(name) => format!("{name}:{}", options.port),
            None => endpoint::line(address, options.port),
        };
        let w = font::text_width(&text, version_scale);
        font::draw(
            &mut canvas,
            &screen_ref(screen),
            screen.width.saturating_sub(w) / 2,
            y,
            &text,
            version_scale,
            INK_DIM,
        );
    }

    canvas
}

/// The geometry the font module needs, without lending it the whole struct.
fn screen_ref(s: &Screen) -> font::Target {
    font::Target {
        width: s.width,
        height: s.height,
        stride: s.stride,
        bytes_per_pixel: s.bytes_per_pixel,
        pixel: s.pixel,
    }
}

/// `Version 0.1.14 — b195b16`, from what the build already knows.
///
/// **Both halves, because neither is enough.** The semver says what release
/// this is and the commit says which build of it — `ADR-027` §3.2.1's whole
/// argument for the compound version, applied to the one line a person reads
/// off a screen.
fn version_line() -> String {
    let mut s = String::new();
    let _ = write!(s, "Version {}", env!("CARGO_PKG_VERSION"));
    if let Some(commit) = option_env!("RIVET_DESCRIBE") {
        let _ = write!(s, " - {commit}");
    }
    s
}

/// Paint every pixel one colour.
fn fill(canvas: &mut [u8], s: &Screen, rgb: [u8; 3]) {
    for y in 0..s.height {
        let row = y * s.stride;
        for x in 0..s.width {
            // `put` drops a write that would run past the end, so the bound
            // is its concern rather than this loop's — and it has to be, now
            // that a pixel is not always four bytes wide.
            s.pixel.put(canvas, row + x * s.bytes_per_pixel, rgb);
        }
    }
}

/// Draw the shield, scaled to `target` pixels square and alpha-blended.
///
/// **Box-averaged rather than nearest-neighbour.** The asset is 512 px and the
/// target is a quarter of a screen — 246 px on the board this was written for —
/// so every output pixel covers about four input ones. Taking one of the four
/// and discarding the rest is what makes a downscaled logo look like it has
/// been through a fax machine.
fn blit_shield(canvas: &mut [u8], s: &Screen, ox: usize, oy: usize, target: usize) {
    if target == 0 {
        return;
    }
    for ty in 0..target {
        for tx in 0..target {
            // The source box this output pixel covers.
            let sx0 = tx * SHIELD_SIZE / target;
            let sx1 = ((tx + 1) * SHIELD_SIZE / target).max(sx0 + 1).min(SHIELD_SIZE);
            let sy0 = ty * SHIELD_SIZE / target;
            let sy1 = ((ty + 1) * SHIELD_SIZE / target).max(sy0 + 1).min(SHIELD_SIZE);

            let (mut r, mut g, mut b, mut a, mut n) = (0u32, 0u32, 0u32, 0u32, 0u32);
            for sy in sy0..sy1 {
                for sx in sx0..sx1 {
                    let at = (sy * SHIELD_SIZE + sx) * 4;
                    r += u32::from(SHIELD[at]);
                    g += u32::from(SHIELD[at + 1]);
                    b += u32::from(SHIELD[at + 2]);
                    a += u32::from(SHIELD[at + 3]);
                    n += 1;
                }
            }
            if n == 0 {
                continue;
            }
            let (r, g, b, a) = (r / n, g / n, b / n, a / n);
            if a == 0 {
                continue;
            }

            let px = ox + tx;
            let py = oy + ty;
            if px >= s.width || py >= s.height {
                continue;
            }
            let at = py * s.stride + px * s.bytes_per_pixel;
            // Source-over, against whatever is already there. On a 16-bit
            // framebuffer "whatever is already there" has been quantised by the
            // write that put it there, which is the right thing to blend
            // against: it is what is actually on the screen.
            let blend = |dst: u8, src: u32| -> u8 {
                let out = (src * a + u32::from(dst) * (255 - a)) / 255;
                u8::try_from(out.min(255)).unwrap_or(255)
            };
            let [dr, dg, db] = s.pixel.get(canvas, at);
            s.pixel
                .put(canvas, at, [blend(dr, r), blend(dg, g), blend(db, b)]);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// The board this was written against, as `sysfs` reports it.
    /// **The screen VC4's KMS driver actually provides** — 16 bits per pixel,
    /// two bytes wide, at the native mode rather than the firmware's
    /// overscanned one. `PLAN.md` §104.13: the splash refused to draw on this
    /// and said so, which was the right refusal and the wrong capability.
    fn kms_board() -> Screen {
        Screen {
            width: 1920,
            height: 1080,
            stride: 1920 * 2,
            bytes_per_pixel: 2,
            pixel: Pixel::Rgb565,
        }
    }

    /// **The regression test for the board.** It renders the whole splash into
    /// a 16-bit framebuffer and asserts the picture is there: the background is
    /// the background, and something that is not the background was drawn in
    /// the middle. Neither was true before 2026-08-31 — the program refused to
    /// render at this depth at all.
    #[test]
    fn the_whole_splash_renders_into_a_sixteen_bit_framebuffer() {
        let s = kms_board();
        let canvas = render(&s, None, &Options::default());
        assert_eq!(canvas.len(), s.stride * s.height);

        let background = {
            let mut one = vec![0u8; s.bytes_per_pixel];
            s.pixel.put(&mut one, 0, BACKGROUND);
            s.pixel.get(&one, 0)
        };
        // A corner is background: the fill reached the far edge of a stride
        // that is no longer four bytes per pixel.
        let corner = s.pixel.get(&canvas, (s.height - 1) * s.stride + (s.width - 1) * s.bytes_per_pixel);
        assert_eq!(corner, background, "the last pixel of the last row");

        // And something was drawn: the shield is a quarter of the height,
        // centred, so the middle of the screen is not background.
        let drawn = (0..s.height)
            .flat_map(|y| (0..s.width).map(move |x| (x, y)))
            .filter(|(x, y)| {
                s.pixel.get(&canvas, y * s.stride + x * s.bytes_per_pixel) != background
            })
            .count();
        assert!(
            drawn > 10_000,
            "only {drawn} pixels differ from the background; the splash did not draw"
        );
    }

    fn board() -> Screen {
        Screen {
            width: 1824,
            height: 984,
            stride: 1824 * 4,
            bytes_per_pixel: 4,
            // What the Pi's driver reports: little-endian XRGB, so blue is
            // byte zero. Asserted here as the SHAPE a test renders in; the
            // program itself reads it from the driver.
            pixel: Pixel::Bytes {
                red: 2,
                green: 1,
                blue: 0,
            },
        }
    }

    /// **The whole point of splitting `render` out**: the layout is checkable
    /// without a framebuffer. Writes a PPM beside the target directory so a
    /// person can look at what a boot would show.
    // -----------------------------------------------------------------
    // The endpoint line — the maintainer's requirement of 2026-08-30
    // -----------------------------------------------------------------

    /// **Both lines are on by default.** The screen is most useful when it says
    /// where to reach the machine; the suppression is for the case where that
    /// is not wanted.
    #[test]
    fn by_default_the_splash_shows_the_version_and_the_endpoint() {
        let o = Options::default();
        assert!(o.version);
        assert!(o.endpoint);
        assert_eq!(o.port, DEFAULT_PORT);
    }

    #[test]
    fn each_line_can_be_suppressed_on_its_own() {
        let o = options(&["--no-endpoint".to_string()]);
        assert!(!o.endpoint);
        assert!(o.version, "suppressing one must not suppress the other");

        let o = options(&["--no-version".to_string()]);
        assert!(!o.version);
        assert!(o.endpoint);

        let o = options(&["--no-version".to_string(), "--no-endpoint".to_string()]);
        assert!(!o.version);
        assert!(!o.endpoint);
    }

    #[test]
    fn the_port_comes_from_the_argument() {
        let o = options(&["--port".to_string(), "9999".to_string()]);
        assert_eq!(o.port, 9999);
    }

    /// **A bad port keeps the default rather than refusing.** This program's
    /// failure mode must never be a boot with no picture.
    #[test]
    fn a_port_that_is_not_a_number_keeps_the_default() {
        assert_eq!(
            options(&["--port".to_string(), "banana".to_string()]).port,
            DEFAULT_PORT
        );
        assert_eq!(options(&["--port".to_string()]).port, DEFAULT_PORT);
    }

    /// **An older pid 1 starting a newer splash is a thing an A/B update
    /// produces**, and the reverse too. An unknown flag is reported and ignored.
    #[test]
    fn an_unknown_argument_is_ignored_rather_than_fatal() {
        let o = options(&["--from-the-future".to_string()]);
        assert_eq!(o, Options::default());
    }

    /// The canvas is still the right size with three lines, which is the thing
    /// that would break if the layout arithmetic went wrong.
    #[test]
    fn a_three_line_splash_still_fills_exactly_one_screen() {
        let s = board();
        let with_endpoint = render(
            &s,
            Some(std::net::Ipv4Addr::new(192, 168, 50, 66)),
            &Options::default(),
        );
        assert_eq!(with_endpoint.len(), s.stride * s.height);
    }

    /// **Suppressing a line changes the picture.** A `--no-version` that drew
    /// the version anyway would be the worst kind of option: one that reports
    /// success and does nothing.
    #[test]
    fn suppressing_the_version_actually_changes_what_is_drawn() {
        let s = board();
        let shown = render(&s, None, &Options::default());
        let hidden = render(
            &s,
            None,
            &Options {
                version: false,
                ..Options::default()
            },
        );
        assert_ne!(
            shown, hidden,
            "--no-version produced an identical screen, so it did nothing"
        );
    }

    /// And so does adding the endpoint.
    #[test]
    fn the_endpoint_line_actually_changes_what_is_drawn() {
        let s = board();
        let without = render(&s, None, &Options::default());
        let with = render(
            &s,
            Some(std::net::Ipv4Addr::new(10, 0, 0, 2)),
            &Options::default(),
        );
        assert_ne!(without, with, "the endpoint line drew nothing");
    }

    /// **The block re-centres when a line goes away**, rather than leaving a
    /// gap where it was. Compared through the shield's vertical position, which
    /// is what the block height moves.
    #[test]
    fn removing_a_line_recentres_the_rest() {
        let s = board();
        let two = render(&s, None, &Options::default());
        let one = render(
            &s,
            None,
            &Options {
                version: false,
                ..Options::default()
            },
        );
        // The first row that differs from a known-background row. Row 0 is
        // above everything drawn, so it IS background -- comparing against it
        // needs no knowledge of how a background pixel is laid out in memory,
        // which an earlier version of this test got wrong.
        let first_ink = |canvas: &[u8]| {
            let background = &canvas[0..s.stride];
            (1..s.height).find(|y| &canvas[y * s.stride..(y + 1) * s.stride] != background)
        };
        assert_ne!(
            first_ink(&two),
            first_ink(&one),
            "the layout did not move when a line was removed"
        );
    }

    #[test]
    fn a_preview_can_be_looked_at() {
        let s = board();
        let canvas = render(&s, None, &Options::default());
        let mut ppm = format!("P6\n{} {}\n255\n", s.width, s.height).into_bytes();
        for y in 0..s.height {
            for x in 0..s.width {
                let rgb = s.pixel.get(&canvas, y * s.stride + x * s.bytes_per_pixel);
                ppm.extend_from_slice(&rgb);
            }
        }
        std::fs::write("/tmp/splash-preview.ppm", &ppm).expect("the preview must be writable");
    }

    /// The shield lands where the maintainer asked: a quarter of the screen's
    /// height, horizontally centred.
    #[test]
    fn the_shield_is_a_quarter_of_the_height_and_centred() {
        let s = board();
        let canvas = render(&s, None, &Options::default());
        let target = s.height / 4;
        assert_eq!(target, 246, "a quarter of 984");
        let background = {
            let mut one = vec![0u8; s.bytes_per_pixel];
            s.pixel.put(&mut one, 0, BACKGROUND);
            s.pixel.get(&one, 0)
        };

        // Find the drawn extent by looking for anything that is not the
        // background, which is what a person would check by eye.
        let (mut min_x, mut max_x) = (usize::MAX, 0usize);
        for y in 0..s.height {
            for x in 0..s.width {
                let at = y * s.stride + x * s.bytes_per_pixel;
                // **Against what the background READS BACK as, not against the
                // constant.** A format that quantises -- RGB565 does -- returns
                // something near `BACKGROUND` rather than `BACKGROUND` itself,
                // and comparing to the constant would report every pixel on the
                // screen as drawn.
                if s.pixel.get(&canvas, at) != background {
                    min_x = min_x.min(x);
                    max_x = max_x.max(x);
                }
            }
        }
        let left = min_x;
        let right = s.width - 1 - max_x;
        assert!(
            left.abs_diff(right) <= 8,
            "not centred: {left} px left, {right} px right"
        );
    }

    /// Nothing is written outside the buffer, whatever the geometry — the
    /// failure that would corrupt memory rather than look wrong.
    #[test]
    fn an_absurdly_small_screen_does_not_write_out_of_bounds() {
        for (w, h) in [(1_usize, 1_usize), (16, 9), (320, 240)] {
            let s = Screen {
                width: w,
                height: h,
                stride: w * 4,
                bytes_per_pixel: 4,
                pixel: Pixel::Bytes {
                    red: 2,
                    green: 1,
                    blue: 0,
                },
            };
            let canvas = render(&s, None, &Options::default());
            assert_eq!(canvas.len(), s.stride * s.height);
        }
    }
}
