//! **A picture of the whole screen** — `ADR-058` §5.
//!
//! # Why this is the compositor's job and not an application's
//!
//! Before `ADR-058` a capture came from whoever held DRM master, which was
//! `rivet-launcher` — so `cargo rivet screenshot` only ever photographed the
//! launcher's own frame and never a lessee's. It looked like a screenshot of
//! the screen and was a screenshot of one program.
//!
//! This is the first program on this machine that can see **the screen**. A
//! per-application hook was considered and rejected on three grounds, and the
//! first is the one that decides it:
//!
//! 1. **It would make the operator's capture depend on the application's
//!    cooperation** — a program could refuse, or be wedged, which is exactly
//!    when a picture is worth most.
//! 2. **It would capture the wrong thing.** An application can only render
//!    what it drew; a screenshot is what is *on the screen*, including what is
//!    behind.
//! 3. **N implementations of one thing** — `ADR-011`. Every kiosk application
//!    would carry an encoder.
//!
//! `wlr-screencopy` remains the right answer for a CLIENT that wants pixels —
//! a recorder, a remote desktop. This is the other question, asked by an
//! operator over the control plane, and its answer must not route through any
//! client at all.
//!
//! # It renders a second time rather than reading back the scanout buffer
//!
//! `DrmCompositor::render_frame` binds the swapchain buffer, draws and unbinds,
//! so there is no bound framebuffer afterwards for `copy_framebuffer` to read.
//! Holding one open across the flip would mean reaching into the compositor's
//! own buffer management for a picture nobody asks for most seconds of most
//! days.
//!
//! So a capture renders the same elements into an offscreen renderbuffer and
//! reads that. It costs one extra frame's work **only when asked**, and it
//! cannot disturb what is on the glass — which for a diagnostic tool is the
//! property that matters: looking must not change what you are looking at.
//!
//! # And the compositor's thread is held only for the GPU's part of it
//!
//! Everything this program does happens on one thread, which also flips the
//! screen and hands clients their buffers back. Taken in one piece, a capture
//! held that thread for about 200 ms — drawing, waiting for the GPU to finish
//! the copy, encoding eight megabytes and writing the file — and for that long
//! nothing on the screen moved and a client drawing at 30 fps ran out of
//! buffers. Looking changed what you were looking at.
//!
//! So the thread does only the drawing. [`draw`] renders the scene into a
//! LINEAR buffer from [`crate::screen::Kms::capture_buffer`] and submits it;
//! [`finish_drawn`], on a thread of its own, has the kernel wait for the GPU's
//! write (`DMA_BUF_IOCTL_SYNC`), reads the pixels straight out of the mapped
//! buffer, encodes them and writes the file. No GL call is made off the
//! compositor's thread.
//!
//! **The linear buffer is the point.** Reading back from the renderer's own
//! offscreen buffer goes through `glReadPixels`, and Mesa's V3D driver
//! untangles its tiled layout on the CPU inside that call: 95 ms on the
//! compositor's thread on a Pi 4, measured, where queueing the copy was meant
//! to cost nothing. A linear buffer needs no untangling, and a mapped buffer
//! needs no GL context to read.
//!
//! **The readback path stays as the fallback**, for a driver that will not
//! render into a linear buffer: [`begin`] draws and copies, and [`collect`]
//! reads the copy [`READ_BACK_AFTER`] later. It holds the thread longer and
//! still does not hold it for the encode and the write.
//!
//! Each piece's time goes in the journal line, so what a capture costs the
//! screen is read off the board rather than asserted here.

use std::time::{Duration, Instant};

use smithay::backend::allocator::dmabuf::{Dmabuf, DmabufMappingMode, DmabufSyncFlags};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::gles::{GlesMapping, GlesRenderbuffer, GlesRenderer};
use smithay::backend::renderer::{Bind, ExportMem, Offscreen, Unbind};
use smithay::desktop::space::{space_render_elements, SpaceRenderElements};
use smithay::output::Output;
use smithay::utils::{Rectangle, Transform};

use crate::state::Rivet;

/// **How long a queued copy is left before it is read back** — two frames and
/// a little over at 60 Hz. Read any sooner and the GPU may still be copying, so
/// the read would wait for it on the compositor's thread, which is the stall
/// this split exists to remove. Nobody asking for a screenshot notices 40 ms.
pub const READ_BACK_AFTER: Duration = Duration::from_millis(40);

/// A capture between [`begin`] and [`collect`]: the copy is queued on the GPU,
/// waiting to be read.
pub struct Pending {
    mapping: GlesMapping,
    width: usize,
    height: usize,
    queued_at: Instant,
    /// How long [`begin`] held the compositor's thread.
    drawing: Duration,
}

impl Pending {
    /// Whether the copy has had [`READ_BACK_AFTER`] to finish.
    #[must_use]
    pub fn ready(&self) -> bool {
        self.queued_at.elapsed() >= READ_BACK_AFTER
    }
}

/// A frame read back into this process's memory, and what taking it cost the
/// compositor's thread. Everything [`finish`] needs, and nothing that must
/// stay on that thread.
pub struct Frame {
    pixels: Vec<u8>,
    width: usize,
    height: usize,
    drawing: Duration,
    reading: Duration,
}

/// What [`finish`] wrote: the answer the caller is sent, and the journal line.
pub struct Finished {
    /// `<path> — <w>x<h>, <n> bytes`, the text a caller has always been sent.
    pub answer: String,
    /// The same, with what each piece of the capture took.
    pub journal: String,
}

/// A capture drawn into a linear buffer and submitted: everything
/// [`finish_drawn`] needs, and nothing that must stay on the compositor's
/// thread.
pub struct Drawn {
    target: Dmabuf,
    width: usize,
    height: usize,
    stride: usize,
    /// How long [`draw`] held the compositor's thread.
    drawing: Duration,
}

/// **On the compositor's thread**: the scene drawn into `target` and the work
/// submitted to the GPU. Nothing here waits for the GPU to finish it.
///
/// # Errors
///
/// Every step names itself — [`begin`]'s reason.
pub fn draw(
    renderer: &mut GlesRenderer,
    output: &Output,
    state: &mut Rivet,
    target: Dmabuf,
) -> Result<Drawn, String> {
    let started = Instant::now();
    let size = output
        .current_mode()
        .ok_or_else(|| "the output has no mode, so there is nothing to photograph".to_string())?
        .size;
    let elements: Vec<SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>> =
        space_render_elements(renderer, [&state.space], output, 1.0)
            .map_err(|e| format!("the scene could not be gathered: {e}"))?;

    renderer
        .bind(target.clone())
        .map_err(|e| format!("the linear buffer could not be bound: {e}"))?;
    // A damage tracker with no history, so the first and only frame it is
    // asked for is a full one — `draw_and_queue` gives the reason. Finishing
    // the frame FLUSHES the work to the GPU and does not wait for it, which is
    // what lets the kernel make [`finish_drawn`] wait instead.
    let mut damage = OutputDamageTracker::from_output(output);
    let drawn = damage
        .render_output(renderer, 0, &elements, crate::screen::CLEAR)
        .map(|_| ())
        .map_err(|e| format!("the scene could not be drawn: {e:?}"));
    let _ = renderer.unbind();
    drawn?;

    let width = size.w.max(0) as usize;
    let stride = target.strides().next().map_or(width * 4, |s| s as usize);
    Ok(Drawn {
        target,
        width,
        height: size.h.max(0) as usize,
        stride,
        drawing: started.elapsed(),
    })
}

/// **Off the compositor's thread**: wait for the GPU's write, read the pixels
/// out of the mapped buffer, then encode and publish them.
///
/// # Errors
///
/// The kernel would not wait on or map the buffer, the buffer is smaller than
/// its own size says, or the file could not be written.
pub fn finish_drawn(drawn: Drawn) -> Result<Finished, String> {
    let waiting = Instant::now();
    drawn
        .target
        .sync_plane(0, DmabufSyncFlags::START | DmabufSyncFlags::READ)
        .map_err(|e| format!("the kernel would not wait for the GPU's write: {e}"))?;
    let waited = waiting.elapsed();

    let reading = Instant::now();
    let pixels = read_rows(&drawn);
    let _ = drawn
        .target
        .sync_plane(0, DmabufSyncFlags::END | DmabufSyncFlags::READ);
    let pixels = pixels?;
    let read = reading.elapsed();

    let (answer, encoded_in, written_in) = encode_and_publish(&pixels, drawn.width, drawn.height)?;
    let journal = format!(
        "{answer}. The compositor's thread was held {} drawing; a thread of its own waited {} \
         for the GPU, read the frame in {}, encoded it in {} and wrote it in {}",
        millis(drawn.drawing),
        millis(waited),
        millis(read),
        millis(encoded_in),
        millis(written_in),
    );
    Ok(Finished { answer, journal })
}

/// The frame's rows, packed, out of the mapped buffer — a row is `width * 4`
/// bytes and the buffer's rows are `stride` apart.
fn read_rows(drawn: &Drawn) -> Result<Vec<u8>, String> {
    let mapping = drawn
        .target
        .map_plane(0, DmabufMappingMode::READ)
        .map_err(|e| format!("the linear buffer could not be mapped: {e}"))?;
    // SAFETY: `map_plane` returned a live read mapping of `length()` bytes,
    // held by `mapping` until the end of this function, and the kernel has
    // been asked to finish every write to it (`finish_drawn`'s sync).
    let bytes = unsafe { std::slice::from_raw_parts(mapping.ptr().cast::<u8>(), mapping.length()) };
    let row = drawn.width * 4;
    let mut pixels = Vec::with_capacity(row * drawn.height);
    for y in 0..drawn.height {
        let start = y * drawn.stride;
        let line = bytes.get(start..start + row).ok_or_else(|| {
            format!(
                "the buffer holds {} bytes, and row {y} of a {}x{} frame at a stride of {} runs \
                 past them",
                bytes.len(),
                drawn.width,
                drawn.height,
                drawn.stride
            )
        })?;
        pixels.extend_from_slice(line);
    }
    Ok(pixels)
}

/// Encode `pixels` and publish them at [`path`]: the answer a caller is sent,
/// and how long each half took.
fn encode_and_publish(
    pixels: &[u8],
    width: usize,
    height: usize,
) -> Result<(String, Duration, Duration), String> {
    let encoding = Instant::now();
    let encoded = crate::qoi::encode(pixels, width, height, width * 4)
        .ok_or_else(|| format!("{width}x{height} could not be encoded"))?;
    let encoded_in = encoding.elapsed();

    let writing = Instant::now();
    let path = path();
    if let Some(parent) = std::path::Path::new(&path).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    publish(&path, &encoded)?;
    let written_in = writing.elapsed();

    Ok((format!("{path} — {width}x{height}, {} bytes", encoded.len()), encoded_in, written_in))
}

/// **The fallback's first piece, on the compositor's thread**: the scene drawn
/// offscreen and a copy of it queued on the GPU — which, on Mesa's V3D driver,
/// also untangles the tiled buffer before it returns (the module note).
///
/// # Errors
///
/// Every step names itself, because a capture that failed silently would be
/// read as *the screen is black* — the one answer a diagnostic tool must never
/// give by accident.
pub fn begin(
    renderer: &mut GlesRenderer,
    output: &Output,
    state: &mut Rivet,
) -> Result<Pending, String> {
    let started = Instant::now();
    let mode = output
        .current_mode()
        .ok_or_else(|| "the output has no mode, so there is nothing to photograph".to_string())?;
    let size = mode.size;

    let elements: Vec<SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>> =
        space_render_elements(renderer, [&state.space], output, 1.0)
            .map_err(|e| format!("the scene could not be gathered: {e}"))?;

    // **`Argb8888`, and the byte order is the whole reason — read it twice.**
    //
    // A DRM fourcc names the pixel as a little-endian word, so `ARGB8888` is
    // `0xAARRGGBB` and lands in memory as **`[B, G, R, A]`**. `crate::qoi`
    // encodes with `px = [chunk[2], chunk[1], chunk[0]]`, which is exactly that
    // order turned back into RGB — because it was written for the DRM canvas,
    // which is the same layout.
    //
    // **The first version of this line asked for `Abgr8888`**, under a comment
    // confidently asserting the opposite, and would have produced a file with
    // red and blue swapped: a picture that looks ALMOST right, which is worse
    // than one that looks broken. Caught by reading `qoi.rs` rather than by
    // looking at a screenshot, which is the only place it would have shown.
    let buffer: GlesRenderbuffer = renderer
        .create_buffer(Fourcc::Argb8888, size.to_logical(1).to_buffer(1, Transform::Normal))
        .map_err(|e| format!("an offscreen buffer could not be created: {e}"))?;

    renderer
        .bind(buffer)
        .map_err(|e| format!("the offscreen buffer could not be bound: {e}"))?;

    // **Unbound whatever happens next**, so a capture that fails half-way
    // cannot leave the offscreen buffer as the renderer's target — the frame
    // loop binds its own again on the next vblank either way.
    let queued = draw_and_queue(renderer, output, &elements, size);
    let _ = renderer.unbind();
    let mapping = queued?;

    Ok(Pending {
        mapping,
        width: size.w.max(0) as usize,
        height: size.h.max(0) as usize,
        queued_at: Instant::now(),
        drawing: started.elapsed(),
    })
}

/// [`begin`]'s drawing, with the offscreen buffer already bound: the scene
/// rendered into it and a copy queued. `copy_framebuffer` issues the read into
/// a pixel buffer and returns; the wait for it is [`collect`]'s.
fn draw_and_queue(
    renderer: &mut GlesRenderer,
    output: &Output,
    elements: &[SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>],
    size: smithay::utils::Size<i32, smithay::utils::Physical>,
) -> Result<GlesMapping, String> {
    // A damage tracker with no history, so the first and only frame it is
    // asked for is a full one. Reusing the frame loop's tracker would hand
    // back *the damage since the last screen frame*, which is usually nothing.
    let mut damage = OutputDamageTracker::from_output(output);
    damage
        .render_output(renderer, 0, elements, crate::screen::CLEAR)
        .map_err(|e| format!("the scene could not be drawn: {e:?}"))?;

    let region = Rectangle::from_size(size.to_logical(1).to_buffer(1, Transform::Normal));
    renderer
        .copy_framebuffer(region, Fourcc::Argb8888)
        .map_err(|e| format!("the frame could not be copied out of the GPU: {e}"))
}

/// **The second piece, on the compositor's thread**, once [`Pending::ready`]
/// says the copy has had time to finish: the pixels moved into memory this
/// process owns, so everything after can leave the thread.
///
/// # Errors
///
/// The copy could not be mapped for reading.
pub fn collect(renderer: &mut GlesRenderer, pending: Pending) -> Result<Frame, String> {
    let started = Instant::now();
    let pixels = renderer
        .map_texture(&pending.mapping)
        .map_err(|e| format!("the copied frame could not be read: {e}"))?
        .to_vec();
    Ok(Frame {
        pixels,
        width: pending.width,
        height: pending.height,
        drawing: pending.drawing,
        reading: started.elapsed(),
    })
}

/// **The third piece, off the compositor's thread**: the frame encoded and
/// published, and a journal line saying what each piece cost.
///
/// # Errors
///
/// The frame could not be encoded, or the file could not be written.
pub fn finish(frame: &Frame) -> Result<Finished, String> {
    let (answer, encoded_in, written_in) =
        encode_and_publish(&frame.pixels, frame.width, frame.height)?;
    let journal = format!(
        "{answer}, by the readback fallback. The compositor's thread was held {} drawing and \
         copying and {} reading the copy; encoding ({}) and writing ({}) ran on a thread of their \
         own",
        millis(frame.drawing),
        millis(frame.reading),
        millis(encoded_in),
        millis(written_in),
    );
    Ok(Finished { answer, journal })
}

/// A duration as milliseconds with one decimal, the unit every line here uses.
fn millis(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

/// `/state/app/rivet-displayd/data/screen.qoi`.
///
/// **Composed from `rivet_core::service`'s own layout**, never spelled: that
/// module owns the shape of an application's data directory and
/// `cargo rivet screenshot` composes the same path from the same function.
/// Two spellings would be a tool that fetches a file nothing writes.
#[must_use]
pub fn path() -> String {
    use rivet_core::service::{data_dir, ServiceName, BUILT_IN_VOLUME};

    match ServiceName::new(rivet_core::compositor::PROVIDER) {
        Some(name) => format!(
            "{}/{}",
            data_dir(BUILT_IN_VOLUME, &name),
            rivet_core::compositor::CAPTURE_FILE
        ),
        // Unreachable — `rivet_core::compositor`'s own test says the provider
        // is a name the supervisor accepts. Composed rather than unwrapped
        // because this crate denies `unwrap` (`ADR-017` §3).
        None => format!(
            "{BUILT_IN_VOLUME}/app/{}/data/{}",
            rivet_core::compositor::PROVIDER,
            rivet_core::compositor::CAPTURE_FILE
        ),
    }
}

/// **Take the published capture away**, because there is no screen for it to
/// be a picture of.
///
/// `cargo rivet screenshot` fetches whatever [`path`] holds, and the file
/// outlives both the screen and the boot. With no display connected, a capture
/// left in place would be fetched as the screen of now — a stale frame
/// presented as a current one. Removed, the fetch says there is no capture,
/// which is true.
pub fn withdraw() {
    match std::fs::remove_file(path()) {
        Ok(()) => println!(
            "rivet-displayd: the last capture was withdrawn — there is no display for it to be a \
             picture of"
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => println!("rivet-displayd: the last capture could not be withdrawn — {e}"),
    }
}

/// Publish `bytes` as the file at `path`, atomically — `ADR-058` §5, B11.
///
/// A plain `fs::write` over the published file truncates it before writing
/// the new bytes, so a control-plane read that lands in between sees a
/// short file, and one that lands over a same-size previous capture can
/// read a mixture of the two — a corrupt image that still decodes, with
/// nothing to say it is wrong. Writing the new bytes to a temporary file
/// and renaming it over `path` avoids both: a rename within one filesystem
/// is atomic, so every reader sees either the complete previous file or the
/// complete new one, never a mixture.
///
/// **The temporary file is a sibling of `path`, and that is load-bearing.**
/// A rename across filesystems is not atomic — it falls back to copying and
/// deleting — so it must land in the same directory `path` does.
///
/// **A temporary file left behind is removed before it is written**, not
/// opened over. One left by an earlier process may belong to another owner,
/// and opening it for writing would be refused where unlinking it from a
/// directory this uid owns is not.
///
/// # Errors
///
/// The temporary file could not be written, or could not be renamed into
/// place.
fn publish(path: &str, bytes: &[u8]) -> Result<(), String> {
    let tmp = format!("{path}.tmp");
    match std::fs::remove_file(&tmp) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("{tmp}: a leftover could not be removed: {e}")),
    }
    std::fs::write(&tmp, bytes).map_err(|e| format!("{tmp}: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{tmp} -> {path}: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// B11: a reader that opens the published path while a new capture is
    /// mid-publish must see either the previous complete bytes or the new
    /// complete bytes — never a partial write. `AGENTS.md` §4 asks for a
    /// regression test at the real boundary rather than a model of it, so
    /// this spins a real writer thread and a real reader thread against
    /// real files in a temporary directory this test mints for itself.
    ///
    /// Every publish writes `size` copies of one marker byte, where `size`
    /// alternates between two values. A torn read — content from mid-write,
    /// or content spliced across two writes — shows up as either a length
    /// this test never wrote, or bytes that do not all match each other.
    #[test]
    fn a_concurrent_reader_never_sees_a_partial_publish() {
        let dir = std::env::temp_dir()
            .join(format!("rivet-displayd-capture-test-{}-{}", std::process::id(), line!()));
        std::fs::create_dir_all(&dir).expect("a temporary directory for this test");
        let path = dir.join("screen.qoi").to_string_lossy().into_owned();

        const SMALL: usize = 64 * 1024;
        const LARGE: usize = 96 * 1024;

        // Seed one complete publish so the reader always has something to
        // open once the race starts.
        publish(&path, &vec![0xAAu8; SMALL]).expect("the seed publish");

        let writer_path = path.clone();
        let writer = std::thread::spawn(move || {
            for i in 0..200u8 {
                let size = if i % 2 == 0 { SMALL } else { LARGE };
                publish(&writer_path, &vec![i; size]).expect("a publish call");
            }
        });

        let reader_path = path.clone();
        let reader = std::thread::spawn(move || {
            for _ in 0..4000 {
                let Ok(bytes) = std::fs::read(&reader_path) else {
                    // A read can lose the race to open the file before the
                    // seed publish's rename; that is not what this test is
                    // checking for.
                    continue;
                };
                assert!(
                    !bytes.is_empty(),
                    "read 0 bytes — no publish in this test ever writes an empty file, so a \
                     torn read was visible"
                );
                assert!(
                    bytes.len() == SMALL || bytes.len() == LARGE,
                    "read {} bytes, a length no publish call wrote — a torn read was visible",
                    bytes.len()
                );
                let marker = bytes[0];
                assert!(
                    bytes.iter().all(|&b| b == marker),
                    "read {} bytes that do not all match their own first byte ({marker}) — a \
                     read spliced across two publishes was visible",
                    bytes.len()
                );
            }
        });

        writer.join().expect("the writer thread");
        reader.join().expect("the reader thread");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **A leftover temporary file is replaced, never written through.**
    ///
    /// The leftover here is a symlink to a file outside the capture, which is
    /// the one shape that tells the two behaviours apart under any uid: opening
    /// it for writing writes the other file, and removing it first leaves that
    /// file alone. A leftover owned by another uid is the case on a board, and
    /// is refused by the open for the same reason it is passed by the unlink.
    #[cfg(unix)]
    #[test]
    fn a_leftover_temporary_file_is_removed_rather_than_written_through() {
        let dir = std::env::temp_dir()
            .join(format!("rivet-displayd-capture-test-{}-{}", std::process::id(), line!()));
        std::fs::create_dir_all(&dir).expect("a temporary directory for this test");
        let path = dir.join("screen.qoi").to_string_lossy().into_owned();
        let elsewhere = dir.join("not-the-capture");
        std::fs::write(&elsewhere, b"untouched").expect("the other file");
        std::os::unix::fs::symlink(&elsewhere, format!("{path}.tmp")).expect("the leftover");

        publish(&path, b"the new capture").expect("a publish over a leftover");

        assert_eq!(std::fs::read(&elsewhere).expect("the other file"), b"untouched");
        assert_eq!(std::fs::read(&path).expect("the capture"), b"the new capture");
        assert!(
            !std::fs::symlink_metadata(&path).expect("the capture").file_type().is_symlink(),
            "the published capture is a file of its own"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
