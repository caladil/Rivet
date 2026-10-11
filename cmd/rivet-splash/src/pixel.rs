//! How a pixel is laid out in the framebuffer — **and why one layout was not
//! enough.**
//!
//! Until 2026-08-31 this program drew 32-bit pixels and refused everything
//! else, with a comment saying that 16-bit framebuffers "exist and pack
//! differently". They do, and one arrived: `PLAN.md` §104.13 put VC4's KMS
//! driver on the board, and the framebuffer it provides is **16 bits per
//! pixel**, so the splash refused to draw and said so.
//!
//! **That refusal was right and is kept.** Writing 32-bit pixels into a 16-bit
//! framebuffer produces colourful noise, which is worse than no splash. What
//! was wrong was having only one format to offer.
//!
//! # The 16 is the driver's, and there is no asking it for another
//!
//! `drivers/gpu/drm/vc4/vc4_drv.c`:
//!
//! ```c
//! drm_fbdev_dma_setup(drm, 16);
//! ```
//!
//! A constant in the driver. `drm_fb_helper` defaults to 32 when a driver
//! passes 0, and VC4 does not pass 0 — so this is not a mode option, not a
//! `video=` suffix, and not something a command line can change. A Rivet that
//! wants a splash on a KMS display has to draw `RGB565`.
//!
//! # The layout is read from the driver, never inferred from the depth
//!
//! `16` does not mean `RGB565`. `RGB555` with a padding bit is also sixteen
//! bits, and drawing one as the other shifts every colour. So
//! [`Pixel::of`] takes the driver's own `fb_bitfield` offsets and lengths and
//! matches on all six numbers; anything it does not recognise is **refused by
//! name**, carrying the numbers, so the next unknown format is one edit away
//! rather than one investigation away.

/// How to read and write one pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pixel {
    /// One whole byte per channel, at these offsets within the pixel. Covers
    /// the 24- and 32-bit layouts, whatever order the driver puts them in.
    Bytes { red: usize, green: usize, blue: usize },
    /// Five bits red, six green, five blue, packed into two little-endian
    /// bytes. Every target this runs on is little-endian (`ADR-003` §4).
    Rgb565,
}

/// One channel's position, as the driver states it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Channel {
    pub offset: u32,
    pub length: u32,
}

impl Pixel {
    /// Decide the layout from what the driver said, or refuse and say what it
    /// said.
    ///
    /// # Errors
    ///
    /// A depth or bitfield arrangement this program cannot draw. **The numbers
    /// travel in the message**, because the alternative is an operator seeing
    /// *"unsupported framebuffer"* and having nothing to act on.
    pub fn of(bits_per_pixel: u32, red: Channel, green: Channel, blue: Channel) -> Result<Self, String> {
        // **Byte-aligned channels first**, because that test is about the
        // channels rather than about the depth: a 24- and a 32-bit pixel differ
        // in how far apart they are, not in how a channel is written.
        if bits_per_pixel >= 24
            && [red, green, blue]
                .iter()
                .all(|c| c.length == 8 && c.offset % 8 == 0)
        {
            return Ok(Self::Bytes {
                red: (red.offset / 8) as usize,
                green: (green.offset / 8) as usize,
                blue: (blue.offset / 8) as usize,
            });
        }
        if bits_per_pixel == 16
            && (red.offset, red.length) == (11, 5)
            && (green.offset, green.length) == (5, 6)
            && (blue.offset, blue.length) == (0, 5)
        {
            return Ok(Self::Rgb565);
        }
        Err(format!(
            "the framebuffer is {bits_per_pixel} bits per pixel with red at {}/{}, green at \
             {}/{} and blue at {}/{} (offset/length in bits), which this program has no way to \
             write. It draws byte-per-channel at 24 and 32 bits, and RGB565 at 16. Refusing \
             rather than writing the wrong shape into it.",
            red.offset, red.length, green.offset, green.length, blue.offset, blue.length
        ))
    }

    /// How many bytes one pixel occupies, when the driver has not said.
    ///
    /// **Only a fallback.** `sysfs` reports `bits_per_pixel` and the caller uses
    /// that; this exists so a `Pixel` is self-describing in a test.
    #[must_use]
    pub const fn bytes(self) -> usize {
        match self {
            Self::Bytes { .. } => 4,
            Self::Rgb565 => 2,
        }
    }

    /// Write one pixel. Out-of-range writes are dropped rather than panicking:
    /// a splash that kills PID 1's child over one edge pixel is worse than a
    /// splash with one wrong pixel.
    pub fn put(self, canvas: &mut [u8], at: usize, rgb: [u8; 3]) {
        match self {
            Self::Bytes { red, green, blue } => {
                let last = at + red.max(green).max(blue);
                if last >= canvas.len() {
                    return;
                }
                canvas[at + red] = rgb[0];
                canvas[at + green] = rgb[1];
                canvas[at + blue] = rgb[2];
            }
            Self::Rgb565 => {
                if at + 1 >= canvas.len() {
                    return;
                }
                let packed = (u16::from(rgb[0] >> 3) << 11)
                    | (u16::from(rgb[1] >> 2) << 5)
                    | u16::from(rgb[2] >> 3);
                canvas[at] = (packed & 0xff) as u8;
                canvas[at + 1] = (packed >> 8) as u8;
            }
        }
    }

    /// Read one pixel back, for alpha blending against what is already there.
    ///
    /// **`RGB565` does not round-trip**, and cannot: five bits cannot hold
    /// eight. The expansion below is the standard one — repeat the high bits
    /// into the low — so that `0x1f` becomes `0xff` rather than `0xf8`, and a
    /// white background reads back as white instead of very slightly grey.
    /// Anything read here has already been quantised by [`Self::put`], so a
    /// blend is against the quantised value, which is what is actually on the
    /// screen.
    #[must_use]
    pub fn get(self, canvas: &[u8], at: usize) -> [u8; 3] {
        match self {
            Self::Bytes { red, green, blue } => {
                let last = at + red.max(green).max(blue);
                if last >= canvas.len() {
                    return [0, 0, 0];
                }
                [canvas[at + red], canvas[at + green], canvas[at + blue]]
            }
            Self::Rgb565 => {
                if at + 1 >= canvas.len() {
                    return [0, 0, 0];
                }
                let packed = u16::from(canvas[at]) | (u16::from(canvas[at + 1]) << 8);
                let r5 = ((packed >> 11) & 0x1f) as u8;
                let g6 = ((packed >> 5) & 0x3f) as u8;
                let b5 = (packed & 0x1f) as u8;
                [(r5 << 3) | (r5 >> 2), (g6 << 2) | (g6 >> 4), (b5 << 3) | (b5 >> 2)]
            }
        }
    }
}

#[cfg(test)]
#[path = "pixel_tests.rs"]
mod tests;
