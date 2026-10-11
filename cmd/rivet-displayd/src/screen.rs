//! **The screen: one device, at most one connector, one crtc, one page flip.**
//!
//! `ADR-058` §2 moved all of this out of every application and into one
//! program. What that bought, measured: an application no longer needs
//! `/usr/lib/gbm`, `/sys/dev/char`, `/sys/devices`, `/sys/class/drm` or
//! `/dev/dri` — §1.1's five grants, paid once here instead of by everything
//! that draws.
//!
//! # The device is held for life; the display comes and goes
//!
//! [`Kms`] is the device, the GBM allocator and the renderer, and it exists
//! whether or not anything is plugged in: a client's dma-buf is imported by the
//! renderer, and a client may connect before any screen does. [`Head`] is the
//! display being driven — a connector, a crtc, a mode and the swapchain that
//! flips onto it — and it is built when a display arrives and dropped when it
//! leaves. [`Kms::rescan`] is the one place either happens, and
//! [`crate::hotplug::decide`] is what it asks.
//!
//! # The board this was written against
//!
//! Under `vc4-kms-v3d` a Raspberry Pi 4 carries **`card0`, `card1` and
//! `renderD128`**, and the display controller and the GPU are *different DRM
//! devices*: VC4 does KMS and cannot render, V3D renders and has no display.
//!
//! **Nothing below encodes which node is which.** [`Kms::open`] asks each
//! `card*` whether it has connectors and prefers one with a display plugged
//! in, because `rivet-long-term-targets` names an x86 machine whose topology
//! is different again, and a constant here would be a Pi fact wearing a
//! platform's clothes.
//!
//! # Mesa is reached through `renderonly`, and that is not our code
//!
//! Mesa's `renderonly` layer bridges V3D and VC4 *inside* the GBM device, so a
//! buffer allocated here is scanout capable and `gbm_bo_get_handle` is the
//! right call. A hand-rolled PRIME export/import across the two devices is
//! the wrong answer and was removed for being one.

use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;

use smithay::backend::allocator::dmabuf::{AsDmabuf, Dmabuf};
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Allocator, Fourcc, Modifier};
use smithay::backend::drm::compositor::{DrmCompositor, FrameFlags};
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmDeviceNotifier};
use smithay::backend::egl::context::EGLContext;
use smithay::backend::egl::display::EGLDisplay;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::Color32F;
use smithay::desktop::space::{space_render_elements, SpaceRenderElements};
use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::drm::control::{connector, crtc, Device as _, Mode, ModeTypeFlags};
use smithay::utils::{DeviceFd, Size, Transform};

use crate::hotplug::{decide, Change, ModeKey, Probe};
use crate::state::Rivet;

/// What the screen is cleared to when nothing is drawing.
///
/// **Not black.** A black screen is indistinguishable from a dead one, and this
/// project has spent real hours on exactly that ambiguity. `rivet-splash` makes
/// the same argument for the same reason.
pub const CLEAR: Color32F = Color32F::new(0.043, 0.055, 0.078, 1.0);

/// The type the whole render path is written in terms of, spelled once.
type Compositor =
    DrmCompositor<GbmAllocator<DrmDeviceFd>, GbmDevice<DrmDeviceFd>, (), DrmDeviceFd>;

/// The GPU, held for the life of the process, and the display it is driving
/// if one is plugged in.
pub struct Kms {
    pub renderer: GlesRenderer,
    /// The display being driven. `None` is an ordinary state on a kiosk —
    /// `.rulings/` R-236 — and every caller treats it as one.
    pub head: Option<Head>,
    /// Held because dropping it drops DRM master.
    device: DrmDevice,
    /// Held for the same reason — and because every allocator made from it
    /// borrows nothing from it, so nothing else keeps it alive.
    /// [`Kms::capture_buffer`] allocates from it too.
    gbm: GbmDevice<DrmDeviceFd>,
    /// Which node this is, for the journal and for an output's model string.
    path: String,
}

/// **One display being driven**: the connector, the crtc, the mode and the
/// swapchain. Built by [`Kms::rescan`] when a display arrives and dropped when
/// it leaves, which clears the crtc.
pub struct Head {
    compositor: Compositor,
    pub output: Output,
    /// What the connector said when this was built, so the next rescan can
    /// tell a spurious event from a change.
    probe: Probe,
    /// True between `queue_frame` and the vblank that answers it. A second
    /// `queue_frame` before that vblank is an error the kernel reports as
    /// `EBUSY`, so the frame loop asks here first.
    pending: bool,
}

/// What a rescan changed, for the Wayland side to follow.
pub enum Changed {
    Nothing,
    /// A display is now driven through this output, replacing any before it.
    Now(Output),
    /// Nothing is driven any more.
    Gone,
}

/// A connector's answer when probed, with what building a head from it needs.
struct Found {
    handle: connector::Handle,
    info: connector::Info,
    mode: Option<Mode>,
}

impl Kms {
    /// Open the DRM device that drives displays and build everything that
    /// draws — **whether or not a display is plugged in**. A device with a
    /// connected connector is preferred; otherwise the first that has
    /// connectors at all is taken and waits for one.
    ///
    /// # Errors
    ///
    /// No device that can drive a display exists, or EGL would not start on
    /// the one that does. Every failure is a `String` naming the device and
    /// the call, because the journal is the only witness a headless board
    /// has. `ADR-017` §3: this program answers rather than panics, all the
    /// way down.
    pub fn open() -> Result<(Self, DrmDeviceNotifier), String> {
        let mut tried = Vec::new();
        let mut waiting: Option<Opened> = None;

        for index in 0..8u8 {
            let path = format!("/dev/dri/card{index}");
            if !std::path::Path::new(&path).exists() {
                continue;
            }
            match Opened::open(&path) {
                Ok(opened) if opened.connected => return Self::build(opened),
                Ok(opened) => {
                    if waiting.is_none() {
                        waiting = Some(opened);
                    }
                }
                Err(why) => tried.push(format!("{path}: {why}")),
            }
        }

        if let Some(opened) = waiting {
            return Self::build(opened);
        }
        if tried.is_empty() {
            return Err("no /dev/dri/card* exists — this machine has no DRM device, or the \
                        supervisor did not grant the node directory"
                .to_string());
        }
        Err(format!(
            "no DRM device can drive a display. Tried: {}",
            tried.join("; ")
        ))
    }

    fn build(opened: Opened) -> Result<(Self, DrmDeviceNotifier), String> {
        let Opened { path, device, notifier, fd, .. } = opened;
        let gbm = GbmDevice::new(fd).map_err(|e| format!("GbmDevice::new: {e}"))?;

        // SAFETY: the GBM device outlives the EGLDisplay — it is moved into
        // `Self::gbm` below and this struct owns both. EGL's own requirement
        // is exactly that the native display outlive the EGLDisplay.
        let egl = unsafe { EGLDisplay::new(gbm.clone()) }
            .map_err(|e| format!("EGLDisplay::new — is /usr/lib/gbm readable AND executable? {e}"))?;
        let context = EGLContext::new(&egl).map_err(|e| format!("EGLContext::new: {e}"))?;

        // SAFETY: the context is moved in and never made current elsewhere;
        // this thread is the only one that ever touches GL.
        let renderer =
            unsafe { GlesRenderer::new(context) }.map_err(|e| format!("GlesRenderer::new: {e}"))?;

        Ok((Self { renderer, head: None, device, gbm, path }, notifier))
    }

    /// The node this compositor drives, for the journal.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// **Ask the connectors what is plugged in, and follow it** — bring a
    /// display up at its preferred mode, take one down, or swap one for
    /// another. [`crate::hotplug::decide`] makes the choice; this carries it
    /// out.
    ///
    /// # Errors
    ///
    /// The device would not list its connectors, or a display that arrived
    /// could not be driven. Either leaves no head: a screen that cannot be
    /// driven is the same to a client as no screen, and the next hot-plug
    /// asks again.
    pub fn rescan(&mut self) -> Result<Changed, String> {
        let found = self.probe()?;
        let probes: Vec<Probe> = found.iter().map(probe_of).collect();
        let driven = self.head.as_ref().map(|h| h.probe);

        let index = match decide(driven, &probes) {
            Change::Keep => return Ok(Changed::Nothing),
            Change::Down => {
                // Dropping the head clears its crtc — `AtomicDrmSurface`'s
                // own `Drop` — so nothing is left scanning out a buffer for a
                // display that is not there.
                self.head = None;
                return Ok(Changed::Gone);
            }
            Change::Up(index) | Change::Swap(index) => index,
        };

        // The old head goes FIRST: the new one may want the same crtc and the
        // same primary plane, and both are claimed until it is dropped.
        let had = self.head.take().is_some();
        let Some(chosen) = found.into_iter().nth(index) else {
            return Ok(if had { Changed::Gone } else { Changed::Nothing });
        };
        match self.bring_up(chosen) {
            Ok(head) => {
                let output = head.output.clone();
                self.head = Some(head);
                Ok(Changed::Now(output))
            }
            Err(why) if had => {
                println!("rivet-displayd: {why}");
                Ok(Changed::Gone)
            }
            Err(why) => Err(why),
        }
    }

    /// Every connector on the device, freshly probed.
    ///
    /// **Forced, which reads the EDID.** The kernel's cached mode list belongs
    /// to whatever was plugged in when it was last probed, so a display that
    /// has just arrived would otherwise be offered the last one's modes.
    fn probe(&self) -> Result<Vec<Found>, String> {
        let resources = self
            .device
            .resource_handles()
            .map_err(|e| format!("{}: resource_handles: {e}", self.path))?;
        Ok(resources
            .connectors()
            .iter()
            .filter_map(|handle| {
                let info = self.device.get_connector(*handle, true).ok()?;
                let mode = if info.state() == connector::State::Connected {
                    preferred(&info)
                } else {
                    None
                };
                Some(Found { handle: *handle, info, mode })
            })
            .collect())
    }

    fn bring_up(&mut self, found: Found) -> Result<Head, String> {
        let Found { handle, info, mode } = found;
        let name = format!("{:?}-{}", info.interface(), info.interface_id());
        let mode = mode.ok_or_else(|| format!("{name} is connected and offers no mode"))?;

        let resources = self
            .device
            .resource_handles()
            .map_err(|e| format!("{}: resource_handles: {e}", self.path))?;
        let crtc = crtc_for(&self.device, &resources, &info)
            .ok_or_else(|| format!("no crtc can drive {name}"))?;

        let surface = self
            .device
            .create_surface(crtc, mode, &[handle])
            .map_err(|e| format!("{name}: create_surface: {e}"))?;

        let (width, height) = mode.size();
        let output = Output::new(
            name.clone(),
            PhysicalProperties {
                size: Size::from((
                    info.size().map_or(0, |s| s.0 as i32),
                    info.size().map_or(0, |s| s.1 as i32),
                )),
                make: "Rivet".into(),
                model: self.path.clone(),
                subpixel: Subpixel::Unknown,
            },
        );
        let output_mode = OutputMode {
            size: Size::from((i32::from(width), i32::from(height))),
            refresh: refresh_millihertz(&mode),
        };
        output.change_current_state(
            Some(output_mode),
            Some(Transform::Normal),
            Some(smithay::output::Scale::Integer(1)),
            Some((0, 0).into()),
        );
        output.set_preferred(output_mode);

        let allocator = GbmAllocator::new(
            self.gbm.clone(),
            // **Both flags, and `SCANOUT` is the one that was wrongly dropped
            // once.** A buffer the compositor renders into is also the buffer
            // the CRTC reads, so asking for rendering alone gets an allocation
            // the kernel will refuse at `AddFB` — late, and with a message
            // about the framebuffer rather than about this line.
            GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
        );

        let renderer_formats = self.renderer.egl_context().dmabuf_render_formats().clone();

        let compositor = DrmCompositor::new(
            &output,
            surface,
            None,
            allocator,
            self.gbm.clone(),
            // XRGB first. `EGL_NATIVE_VISUAL_ID` on this board answers XR24,
            // and offering AR30/XR30 ahead of it got a config that scanned out
            // as garbage — measured, not assumed.
            [Fourcc::Xrgb8888, Fourcc::Argb8888],
            renderer_formats,
            self.device.cursor_size(),
            Some(self.gbm.clone()),
        )
        .map_err(|e| format!("{name}: DrmCompositor::new: {e}"))?;

        let probe = Probe { connector: u32::from(handle), mode: Some(key_of(&mode)) };
        Ok(Head { compositor, output, probe, pending: false })
    }

    /// Draw what the space holds and queue the flip.
    ///
    /// Returns `Ok(true)` when a frame was queued and a vblank is therefore
    /// coming. `Ok(false)` means nothing was submitted — nothing changed, a
    /// flip is still pending, or **there is no display**, none of which is an
    /// error: a kiosk whose content is static legitimately produces no frames
    /// at all, and a loop that retried on it would spin a core for nothing.
    ///
    /// # Errors
    ///
    /// A render or a flip that the kernel refused.
    pub fn frame(&mut self, state: &mut Rivet) -> Result<bool, String> {
        let Some(head) = self.head.as_mut() else {
            return Ok(false);
        };
        if head.pending {
            return Ok(false);
        }

        let elements: Vec<
            SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>,
        > = space_render_elements(&mut self.renderer, [&state.space], &head.output, 1.0)
            .map_err(|e| format!("space_render_elements: {e}"))?;

        let result = head
            .compositor
            .render_frame(&mut self.renderer, &elements, CLEAR, FrameFlags::DEFAULT)
            .map_err(|e| format!("render_frame: {e}"))?;

        if result.is_empty {
            return Ok(false);
        }

        head.compositor
            .queue_frame(())
            .map_err(|e| format!("queue_frame: {e}"))?;
        head.pending = true;
        Ok(true)
    }

    /// **A buffer to photograph the screen into**, LINEAR, so a thread with no
    /// GL context can read it straight out of memory once the kernel says the
    /// GPU has finished writing it — `crate::capture`'s module note says why
    /// that matters. One per capture: captures are rare, and a buffer kept
    /// between them would need a rule about who may draw into it while a
    /// worker is still reading the last one.
    ///
    /// # Errors
    ///
    /// There is no display, or GBM would not allocate or export a linear
    /// buffer of its size.
    pub fn capture_buffer(&self) -> Result<Dmabuf, String> {
        let size = self
            .head
            .as_ref()
            .and_then(|h| h.output.current_mode())
            .ok_or_else(|| NO_DISPLAY.to_string())?
            .size;
        let width = u32::try_from(size.w).map_err(|_| format!("a width of {} is not a size", size.w))?;
        let height = u32::try_from(size.h).map_err(|_| format!("a height of {} is not a size", size.h))?;
        let mut allocator = GbmAllocator::new(
            self.gbm.clone(),
            GbmBufferFlags::RENDERING | GbmBufferFlags::LINEAR,
        );
        let buffer = allocator
            .create_buffer(width, height, Fourcc::Argb8888, &[Modifier::Linear])
            .map_err(|e| format!("a linear {width}x{height} buffer could not be allocated: {e}"))?;
        buffer
            .export()
            .map_err(|e| format!("the linear buffer could not be exported as a dma-buf: {e}"))
    }

    /// A vblank arrived on `crtc`: the queued frame is on the glass.
    ///
    /// **A vblank for a crtc no head is driving is ignored.** One can arrive
    /// after its head was dropped by an unplug, and handing it to a head built
    /// since would tell that head a flip it never queued had completed.
    ///
    /// # Errors
    ///
    /// The compositor rejected the submission — a device that went away under
    /// us is the realistic cause.
    pub fn submitted(&mut self, crtc: crtc::Handle) -> Result<(), String> {
        let Some(head) = self.head.as_mut() else {
            return Ok(());
        };
        if head.compositor.crtc() != crtc || !head.pending {
            return Ok(());
        }
        head.pending = false;
        head.compositor
            .frame_submitted()
            .map(|_| ())
            .map_err(|e| format!("frame_submitted: {e}"))
    }
}

/// What a capture says when there is nothing to photograph.
pub const NO_DISPLAY: &str =
    "no display is connected, so there is nothing to photograph; plug one in and ask again";

/// One DRM node, opened and asked whether it can drive a display.
struct Opened {
    path: String,
    device: DrmDevice,
    notifier: DrmDeviceNotifier,
    fd: DrmDeviceFd,
    /// Something is plugged into one of its connectors right now.
    connected: bool,
}

impl Opened {
    fn open(path: &str) -> Result<Self, String> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(path)
            .map_err(|e| format!("open: {e}"))?;

        // SAFETY: `into_raw_fd` gives up ownership of a descriptor this
        // function just opened and has not duplicated, and `OwnedFd` takes it.
        // No other owner exists between the two statements.
        let owned = unsafe { OwnedFd::from_raw_fd(std::os::fd::IntoRawFd::into_raw_fd(file)) };
        let fd = DrmDeviceFd::new(DeviceFd::from(owned));

        // `true` disables connectors that are not part of a surface, which is
        // what stops a mode left behind by the firmware from staying lit.
        //
        // **It costs one kernel WARNING per boot on this board, and it is kept
        // anyway:**
        //
        //     WARNING: CPU: 3 PID: 154 at drivers/gpu/drm/vc4/vc4_hvs.c:1064
        //     __vc4_hvs_stop_channel+0x158/0x180
        //     Comm: rivet-displayd … vc4_crtc_atomic_disable ← disable_outputs
        //
        // It is a `WARN_ON` in the VC4 driver about the state of an HVS channel
        // it is being asked to stop — reached because the Raspberry Pi firmware
        // has already configured the display by the time Linux starts, so the
        // first atomic commit that disables outputs is stopping a channel the
        // driver did not start. **A warning, not an oops**: the machine carries
        // on and the modeset that follows succeeds.
        //
        // Passing `false` would remove the warning and reintroduce the fault it
        // guards against — a connector the firmware lit staying lit under a
        // compositor that is not driving it. A cosmetic kernel line is the
        // cheaper of the two.
        let (device, notifier) =
            DrmDevice::new(fd.clone(), true).map_err(|e| format!("DrmDevice::new: {e}"))?;

        let resources = device
            .resource_handles()
            .map_err(|e| format!("resource_handles: {e}"))?;
        if resources.connectors().is_empty() {
            return Err("it has no connectors, so it drives no display".to_string());
        }
        let connected = resources.connectors().iter().any(|handle| {
            device
                .get_connector(*handle, true)
                .is_ok_and(|info| info.state() == connector::State::Connected)
        });

        Ok(Self { path: path.to_string(), device, notifier, fd, connected })
    }
}

/// The mode a connector would be driven at: its preferred one, or the first it
/// offers.
fn preferred(info: &connector::Info) -> Option<Mode> {
    info.modes()
        .iter()
        .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
        .or_else(|| info.modes().first())
        .copied()
}

fn probe_of(found: &Found) -> Probe {
    Probe { connector: u32::from(found.handle), mode: found.mode.as_ref().map(key_of) }
}

fn key_of(mode: &Mode) -> ModeKey {
    let (width, height) = mode.size();
    ModeKey { width, height, refresh: refresh_millihertz(mode) }
}

/// The first crtc that any of this connector's encoders can use.
fn crtc_for(
    device: &DrmDevice,
    resources: &smithay::reexports::drm::control::ResourceHandles,
    info: &connector::Info,
) -> Option<crtc::Handle> {
    info.encoders()
        .iter()
        .filter_map(|handle| device.get_encoder(*handle).ok())
        .flat_map(|encoder| resources.filter_crtcs(encoder.possible_crtcs()))
        .next()
}

/// A mode's refresh rate in millihertz, which is what `Output` wants.
///
/// The arithmetic is DRM's own — `drm_mode_vrefresh` — including the
/// interlace and double-scan adjustments, because a mode that carries either
/// and is reported at half or twice its rate makes every frame-pacing number
/// downstream wrong by that factor.
fn refresh_millihertz(mode: &Mode) -> i32 {
    let clock = u64::from(mode.clock());
    let htotal = u64::from(mode.hsync().2);
    let vtotal = u64::from(mode.vsync().2);
    if htotal == 0 || vtotal == 0 {
        return 60_000;
    }

    let mut refresh = (clock * 1_000_000) / (htotal * vtotal);
    let flags = mode.flags();
    if flags.contains(smithay::reexports::drm::control::ModeFlags::INTERLACE) {
        refresh *= 2;
    }
    if flags.contains(smithay::reexports::drm::control::ModeFlags::DBLSCAN) {
        refresh /= 2;
    }
    let divisor = u64::from(mode.vscan());
    if divisor > 1 {
        refresh /= divisor;
    }

    i32::try_from(refresh).unwrap_or(60_000)
}
