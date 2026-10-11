//! What each layout writes, and the one it refuses.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

fn ch(offset: u32, length: u32) -> Channel {
    Channel { offset, length }
}

/// The layout VC4's KMS framebuffer actually reports — `PLAN.md` §104.13, and
/// the reason this module exists.
#[test]
fn a_sixteen_bit_five_six_five_framebuffer_is_recognised() {
    let got = Pixel::of(16, ch(11, 5), ch(5, 6), ch(0, 5)).expect("565");
    assert_eq!(got, Pixel::Rgb565);
    assert_eq!(got.bytes(), 2);
}

/// The layout the firmware framebuffer reports — little-endian `XRGB`, so red
/// is byte 2. This is what the program drew before and must keep drawing.
#[test]
fn a_thirty_two_bit_byte_per_channel_framebuffer_keeps_its_byte_offsets() {
    let got = Pixel::of(32, ch(16, 8), ch(8, 8), ch(0, 8)).expect("XRGB");
    assert_eq!(
        got,
        Pixel::Bytes {
            red: 2,
            green: 1,
            blue: 0
        }
    );
}

/// 24-bit packs the same way, one byte closer together — and the test is about
/// the CHANNELS, not the depth, which is why both reach the same arm.
#[test]
fn a_twenty_four_bit_framebuffer_is_the_same_channels_in_a_smaller_pixel() {
    let got = Pixel::of(24, ch(16, 8), ch(8, 8), ch(0, 8)).expect("RGB888");
    assert!(matches!(got, Pixel::Bytes { red: 2, .. }));
}

/// **`16` does not mean `RGB565`.** `RGB555` with a padding bit is also sixteen
/// bits and shifts every colour if drawn as 565, so it is refused rather than
/// guessed — and the refusal carries the numbers the driver gave.
#[test]
fn a_sixteen_bit_five_five_five_framebuffer_is_refused_with_its_numbers() {
    let why = Pixel::of(16, ch(10, 5), ch(5, 5), ch(0, 5)).expect_err("not 565");
    assert!(why.contains("16 bits"), "{why}");
    assert!(
        why.contains("10/5") && why.contains("5/5"),
        "the driver's own numbers must travel: {why}"
    );
    assert!(
        why.contains("RGB565"),
        "the refusal must say what it CAN draw: {why}"
    );
}

/// A depth nothing here can write is refused, and the message is actionable
/// rather than a shrug.
#[test]
fn an_eight_bit_paletted_framebuffer_is_refused() {
    let why = Pixel::of(8, ch(0, 8), ch(0, 8), ch(0, 8)).expect_err("paletted");
    assert!(why.contains('8'), "{why}");
}

/// **Every 565 value round-trips to itself.** `put` then `get` must be stable,
/// or the alpha blend in the shield walks the colour a little further from the
/// truth on every overlapping draw.
#[test]
fn writing_and_reading_back_a_565_pixel_is_stable() {
    let mut buf = [0u8; 4];
    for rgb in [[0, 0, 0], [255, 255, 255], [37, 42, 200], [8, 130, 250]] {
        Pixel::Rgb565.put(&mut buf, 0, rgb);
        let once = Pixel::Rgb565.get(&buf, 0);
        Pixel::Rgb565.put(&mut buf, 0, once);
        assert_eq!(
            Pixel::Rgb565.get(&buf, 0),
            once,
            "{rgb:?} was not stable across a second write"
        );
    }
}

/// **White stays white and black stays black**, which is the property the
/// expansion is chosen for: a five-bit `0x1f` must read back `0xff`, not
/// `0xf8`, or a white background comes back very slightly grey and the shield
/// blends against the wrong thing.
#[test]
fn the_extremes_survive_565_exactly() {
    let mut buf = [0u8; 2];
    Pixel::Rgb565.put(&mut buf, 0, [255, 255, 255]);
    assert_eq!(Pixel::Rgb565.get(&buf, 0), [255, 255, 255]);
    Pixel::Rgb565.put(&mut buf, 0, [0, 0, 0]);
    assert_eq!(Pixel::Rgb565.get(&buf, 0), [0, 0, 0]);
}

/// 565 keeps the channels apart — a red pixel must not come back with green or
/// blue in it, which is the failure a wrong shift produces and which looks
/// plausible on a screen.
#[test]
fn the_channels_do_not_bleed_into_each_other() {
    let mut buf = [0u8; 2];
    Pixel::Rgb565.put(&mut buf, 0, [255, 0, 0]);
    assert_eq!(Pixel::Rgb565.get(&buf, 0), [255, 0, 0]);
    Pixel::Rgb565.put(&mut buf, 0, [0, 255, 0]);
    assert_eq!(Pixel::Rgb565.get(&buf, 0), [0, 255, 0]);
    Pixel::Rgb565.put(&mut buf, 0, [0, 0, 255]);
    assert_eq!(Pixel::Rgb565.get(&buf, 0), [0, 0, 255]);
}

/// **A write past the end is dropped, not a panic.** This runs as PID 1's
/// child on a machine with no shell; killing it over one edge pixel is worse
/// than one wrong pixel.
#[test]
fn a_write_past_the_end_of_the_canvas_is_dropped_rather_than_panicking() {
    let mut buf = [0u8; 2];
    Pixel::Rgb565.put(&mut buf, 1, [255, 0, 0]);
    assert_eq!(buf, [0, 0], "nothing may have been written");
    assert_eq!(Pixel::Rgb565.get(&buf, 8), [0, 0, 0]);

    let mut wide = [0u8; 4];
    let bytes = Pixel::Bytes {
        red: 2,
        green: 1,
        blue: 0,
    };
    bytes.put(&mut wide, 2, [255, 255, 255]);
    assert_eq!(wide, [0, 0, 0, 0], "nothing may have been written");
}
