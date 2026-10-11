//! **The three backgrounds R-207 ships on the GPU**, drawn through
//! `rivetos_addon::gl`.
//!
//! # What moved and what did not
//!
//! **The simulation did not move.** Contours' height field, Life's automaton
//! and Boxes' palette are still stepped on the CPU by [`crate::designed`] and
//! [`crate::background`], by the same code, from the same seed, at the same
//! rate. Every one of them is a small grid updated a few times a second —
//! 96x54, 160x90, 8x8 — and none is where the time went.
//!
//! **The RASTERISATION moved**, which is the whole of the measurement in
//! `GPU-Backgrounds` §1.3:
//!
//! | background | CPU | GPU |
//! | :-- | --: | --: |
//! | contours/ice-blue | 13.34 ms | **8.19 ms** |
//! | life/pulse | 11.69 ms | **9.80 ms** |
//! | boxes/ice-blue | 11.38 ms | **8.93 ms** |
//!
//! Each frame the grid is uploaded as a small texture — 5,184 bytes for
//! Contours, 28,800 for Life, 192 for Boxes — and one full-screen fragment
//! shader turns it into two million pixels. That is the trade: a few kilobytes
//! across the bus against 8 MB of `draw::blend` on four ARM cores.
//!
//! # The floor these numbers sit on
//!
//! Starfield measured **3.86 ms on the GPU against 4.11 on the CPU**, which is
//! a tie, and R-207 keeps it on the CPU because of it. That 3.86 is the V3D's
//! cost for a frame with almost no fragment work at all, so it is the floor
//! every row above is standing on: Contours' shader costs about 4 ms and its
//! surface costs the other 4. **The GPU is worth it only where there is
//! per-pixel work to move.** It is headroom, not rescue, and this module
//! exists for the three backgrounds that had some.
//!
//! # Why the CPU paths stay
//!
//! R-206 D3. The x86 machine in this project's long-term targets has no V3D,
//! and a pack that deleted its CPU rasterisers would leave it with nothing to
//! draw. All three backgrounds here declare [`rivetos_addon::Drawn::Either`]
//! and the host picks.

// **The second of the crate's two exceptions to `deny(unsafe_code)`** — see
// `lib.rs`, which argues the narrowing. Every `unsafe` below is a call through
// a function pointer `crate::gles` resolved; no pointer the host owns is
// dereferenced, because the GPU boundary hands over integers rather than
// memory.
#![allow(unsafe_code)]

use core::ffi::c_char;

use crate::gles::{gles, Gles};

// ---------------------------------------------------------------------------
// The GLES constants this module names
// ---------------------------------------------------------------------------
//
// Spelled here rather than pulled from a binding crate for the reason
// `examples/probes/gpu-surface` gives: a dozen integers from a specification
// that has not changed since 2008 is a smaller thing to own than a dependency,
// and every one of them can be checked against the header by eye.

const GL_FRAGMENT_SHADER: u32 = 0x8B30;
const GL_VERTEX_SHADER: u32 = 0x8B31;
const GL_COMPILE_STATUS: u32 = 0x8B81;
const GL_LINK_STATUS: u32 = 0x8B82;
const GL_INFO_LOG_LENGTH: u32 = 0x8B84;
const GL_ARRAY_BUFFER: u32 = 0x8892;
const GL_STATIC_DRAW: u32 = 0x88E4;
const GL_FLOAT: u32 = 0x1406;
const GL_TRIANGLE_STRIP: u32 = 0x0005;
const GL_TEXTURE_2D: u32 = 0x0DE1;
const GL_TEXTURE0: u32 = 0x84C0;
const GL_LUMINANCE: u32 = 0x1909;
const GL_LUMINANCE_ALPHA: u32 = 0x190A;
const GL_RGB: u32 = 0x1907;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_TEXTURE_MIN_FILTER: u32 = 0x2801;
const GL_TEXTURE_MAG_FILTER: u32 = 0x2800;
const GL_TEXTURE_WRAP_S: u32 = 0x2802;
const GL_TEXTURE_WRAP_T: u32 = 0x2803;
const GL_NEAREST: i32 = 0x2600;
const GL_LINEAR: i32 = 0x2601;
const GL_CLAMP_TO_EDGE: i32 = 0x812F;
const GL_UNPACK_ALIGNMENT: u32 = 0x0CF5;

// ---------------------------------------------------------------------------
// The shaders
// ---------------------------------------------------------------------------

/// A full-screen triangle strip in clip space, with texture coordinates
/// alongside. One buffer, shared by all three programs.
const QUAD: [f32; 16] = [
    -1.0, -1.0, 0.0, 1.0, //
    1.0, -1.0, 1.0, 1.0, //
    -1.0, 1.0, 0.0, 0.0, //
    1.0, 1.0, 1.0, 0.0,
];

const VERTEX_SRC: &str = "attribute vec2 position;\n\
                          attribute vec2 uv_in;\n\
                          varying vec2 uv;\n\
                          void main() {\n\
                          \x20 uv = uv_in;\n\
                          \x20 gl_Position = vec4(position, 0.0, 1.0);\n\
                          }\n";

/// **The whole of Contours, in eight lines.**
///
/// `texture2D` with `GL_LINEAR` is the bilinear interpolation
/// [`crate::designed`]'s `AxisMap` and run loop spend their time on.
/// `fwidth(v)` is the local gradient its longest comment is about — the change
/// in `v` between adjacent fragments, which the CPU version computes by
/// sampling neighbours. The rest is the same compare and the same coverage
/// ramp, constant for constant.
///
/// `GL_OES_standard_derivatives` is what `fwidth` needs in ES 2.0. It is
/// present on the V3D; a machine without it is **refused by name at compile**
/// rather than quietly drawing something subtly wrong.
const CONTOURS_FRAGMENT_SRC: &str =
    "#extension GL_OES_standard_derivatives : require\n\
     precision highp float;\n\
     varying vec2 uv;\n\
     uniform sampler2D field;\n\
     uniform float levels;\n\
     uniform vec3 line_colour;\n\
     uniform float half_width;\n\
     void main() {\n\
     \x20 float v = texture2D(field, uv).r * levels;\n\
     \x20 float distance = abs(v - floor(v + 0.5));\n\
     \x20 float threshold = half_width * fwidth(v);\n\
     \x20 float coverage = 1.0 - min(distance / max(threshold, 1e-6), 1.0);\n\
     \x20 gl_FragColor = vec4(line_colour * coverage, 1.0);\n\
     }\n";

/// **Life, as one fetch and a distance.**
///
/// `floor(uv * grid)` finds the cell; the CPU version walks a bounding box per
/// warm cell and calls `circle_alpha` inside it, which is why the fading trail
/// costs more than the live population. Here the trail is free: the shader
/// visits every pixel exactly once whatever the board looks like.
///
/// `texel.r` is the heat and `texel.a` the pulse fraction — one
/// `GL_LUMINANCE_ALPHA` fetch carrying both, which is why the board uploads as
/// two bytes a cell rather than one texture each.
const LIFE_FRAGMENT_SRC: &str =
    "precision highp float;\n\
     varying vec2 uv;\n\
     uniform sampler2D board;\n\
     uniform vec2 grid;\n\
     uniform vec2 cell_px;\n\
     uniform float full_radius;\n\
     uniform vec3 cell_colour;\n\
     void main() {\n\
     \x20 vec2 scaled = uv * grid;\n\
     \x20 vec2 cell = floor(scaled);\n\
     \x20 vec4 texel = texture2D(board, (cell + 0.5) / grid);\n\
     \x20 vec2 offset = (scaled - cell - 0.5) * cell_px;\n\
     \x20 float radius = texel.a * full_radius;\n\
     \x20 float coverage = clamp(radius - length(offset) + 0.5, 0.0, 1.0);\n\
     \x20 gl_FragColor = vec4(cell_colour * (texel.r * coverage), 1.0);\n\
     }\n";

/// **Boxes, and the one subtlety in this file.**
///
/// The cell is found from `uv * viewport / cell_px` and **not** from
/// `uv * grid`, because [`crate::background`]'s `Boxes` computes its cell size
/// by integer division — `width / 8` — and leaves whatever remains at the
/// right and bottom edges **unpainted**. A surface 1920 wide divides evenly and
/// a surface 1921 does not; dividing `uv` by eight would stretch eight cells
/// across the whole width and paint the remainder, which is a different
/// picture from the one the CPU path draws and the one the baseline recorded.
///
/// `step(cell, grid - 1.0)` is that remainder: a fragment past the last whole
/// cell multiplies out to black, which is what an unpainted pixel is on an
/// opaque surface that had no clear.
const BOXES_FRAGMENT_SRC: &str = "precision highp float;\n\
                                  varying vec2 uv;\n\
                                  uniform sampler2D cells;\n\
                                  uniform vec2 grid;\n\
                                  uniform vec2 cell_px;\n\
                                  uniform vec2 viewport;\n\
                                  void main() {\n\
                                  \x20 vec2 cell = floor(uv * viewport / cell_px);\n\
                                  \x20 vec2 inside = step(cell, grid - 1.0);\n\
                                  \x20 vec2 at = (min(cell, grid - 1.0) + 0.5) / grid;\n\
                                  \x20 vec3 rgb = texture2D(cells, at).rgb;\n\
                                  \x20 gl_FragColor = vec4(rgb * inside.x * inside.y, 1.0);\n\
                                  }\n";

// ---------------------------------------------------------------------------
// What a background hands this module
// ---------------------------------------------------------------------------

/// **One frame's worth of state, ready to upload.**
///
/// Built by [`crate::background::Screensaver::uploaded`] out of the simulation
/// the CPU has just stepped. The borrow is of a scratch buffer the background
/// owns and refills, so a frame costs no allocation after the first.
///
/// **This is the seam that keeps GL out of the simulation.** `designed.rs`
/// knows about heat and pulse fractions and knows nothing about textures;
/// this module knows about textures and nothing about Conway.
pub(crate) enum Uploaded<'a> {
    Contours {
        /// `FIELD_W * FIELD_H` luminance bytes.
        field: &'a [u8],
        width: i32,
        height: i32,
        levels: f32,
        half_width: f32,
        colour: [f32; 3],
    },
    Life {
        /// `cols * rows` luminance-alpha pairs: heat, then pulse fraction.
        board: &'a [u8],
        cols: f32,
        rows: f32,
        cell_px: (f32, f32),
        full_radius: f32,
        colour: [f32; 3],
    },
    Boxes {
        /// `cols * rows` RGB triples.
        cells: &'a [u8],
        cols: f32,
        rows: f32,
        /// **Truncated**, from `width / cols`. See [`BOXES_FRAGMENT_SRC`].
        cell_px: (f32, f32),
    },
}

impl Uploaded<'_> {
    /// Which program draws this.
    const fn program(&self) -> Program {
        match self {
            Self::Contours { .. } => Program::Contours,
            Self::Life { .. } => Program::Life,
            Self::Boxes { .. } => Program::Boxes,
        }
    }
}

/// The three programs, as a key into [`Gpu`]'s cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Program {
    Contours,
    Life,
    Boxes,
}

impl Program {
    const fn fragment_src(self) -> &'static str {
        match self {
            Self::Contours => CONTOURS_FRAGMENT_SRC,
            Self::Life => LIFE_FRAGMENT_SRC,
            Self::Boxes => BOXES_FRAGMENT_SRC,
        }
    }

    /// The index this program occupies in [`Gpu::built`].
    const fn slot(self) -> usize {
        match self {
            Self::Contours => 0,
            Self::Life => 1,
            Self::Boxes => 2,
        }
    }
}

// ---------------------------------------------------------------------------
// The GL objects
// ---------------------------------------------------------------------------

/// One compiled program and the uniform locations it uses.
///
/// A location of `-1` is what `glGetUniformLocation` answers for a name the
/// program does not declare, and `glUniform*` with `-1` is a **defined
/// no-op** — so nothing here has to be conditional on which program is bound.
#[derive(Debug, Clone, Copy)]
struct Built {
    program: u32,
    u_field: i32,
    u_levels: i32,
    u_colour: i32,
    u_half_width: i32,
    u_board: i32,
    u_grid: i32,
    u_cell_px: i32,
    u_full_radius: i32,
    u_cell_colour: i32,
    u_cells: i32,
    u_viewport: i32,
}

/// **This pack's GL objects, for the life of the host's context.**
///
/// Created on the first GPU frame and kept. Nothing here is ever deleted,
/// which is correct rather than lazy: the context belongs to the host and
/// outlives any one background, and a pack that deleted its program when the
/// operator changed background would recompile it when they changed back.
///
/// Three programs, one quad and one texture is the whole inventory.
pub(crate) struct Gpu {
    built: [Option<Built>; 3],
    quad: u32,
    texture: u32,
}

impl Gpu {
    /// Set up the quad and the texture. The programs are compiled on demand.
    ///
    /// # Errors
    /// The resolver's sentence, when this process has no GLES — see
    /// [`crate::gles`].
    pub(crate) fn new() -> Result<Self, String> {
        let gl = gles()?;
        let mut quad = 0u32;
        let mut texture = 0u32;
        // SAFETY: the host's context is current — that is `rivetos_addon::gl`'s
        // contract for the call this runs inside — and every argument below is
        // a local of the right type.
        unsafe {
            (gl.glGenBuffers)(1, &raw mut quad);
            (gl.glBindBuffer)(GL_ARRAY_BUFFER, quad);
            (gl.glBufferData)(
                GL_ARRAY_BUFFER,
                std::mem::size_of_val(&QUAD) as isize,
                QUAD.as_ptr().cast(),
                GL_STATIC_DRAW,
            );
            (gl.glGenTextures)(1, &raw mut texture);
            (gl.glBindTexture)(GL_TEXTURE_2D, texture);
            (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
            // **One byte of row alignment.** The default is four, and every
            // texture here is one, two or three bytes a texel over a width
            // that is not a multiple of four — 96 luminance bytes is, 8 RGB
            // triples is not. Getting this wrong skews every row after the
            // first, which looks like a diagonal smear rather than like a
            // bug in a constant.
            (gl.glPixelStorei)(GL_UNPACK_ALIGNMENT, 1);
        }
        Ok(Self {
            built: [None; 3],
            quad,
            texture,
        })
    }

    /// Compile and link one program, or hand back the one already built.
    ///
    /// # Errors
    /// The compiler's or linker's own log, **verbatim**. A shader that will
    /// not build on this driver is the most likely failure here and the least
    /// guessable, so nothing is summarised away.
    fn build(&mut self, gl: &Gles, which: Program) -> Result<Built, String> {
        if let Some(built) = self.built[which.slot()] {
            return Ok(built);
        }
        // SAFETY: the context is current; every object below is created here
        // and every pointer is to a local that outlives its call.
        let built = unsafe {
            let vertex = compile(gl, GL_VERTEX_SHADER, VERTEX_SRC)?;
            let fragment = compile(gl, GL_FRAGMENT_SHADER, which.fragment_src())?;
            let program = (gl.glCreateProgram)();
            (gl.glAttachShader)(program, vertex);
            (gl.glAttachShader)(program, fragment);
            (gl.glLinkProgram)(program);
            let mut ok: i32 = 0;
            (gl.glGetProgramiv)(program, GL_LINK_STATUS, &raw mut ok);
            if ok == 0 {
                return Err(format!(
                    "the {which:?} program would not link: {}",
                    program_log(gl, program)
                ));
            }
            (gl.glUseProgram)(program);

            // **Every attribute slot off before this program turns its own
            // on.** Three programs are built in one context over a session and
            // nothing is deleted, so a slot a *previous* program enabled is
            // still enabled when this one links. ES 2.0 guarantees at least
            // eight slots.
            for slot in 0..8 {
                (gl.glDisableVertexAttribArray)(slot);
            }

            let stride = 4 * std::mem::size_of::<f32>() as i32;
            let float = std::mem::size_of::<f32>();
            (gl.glBindBuffer)(GL_ARRAY_BUFFER, self.quad);
            let position = (gl.glGetAttribLocation)(program, c"position".as_ptr());
            let uv_in = (gl.glGetAttribLocation)(program, c"uv_in".as_ptr());
            if position < 0 || uv_in < 0 {
                return Err(format!(
                    "the {which:?} program declares no position/uv_in attribute pair, which \
                     means the vertex shader that linked is not the one in this file"
                ));
            }
            #[allow(clippy::cast_sign_loss)]
            let (position, uv_in) = (position as u32, uv_in as u32);
            (gl.glEnableVertexAttribArray)(position);
            (gl.glVertexAttribPointer)(
                position,
                2,
                GL_FLOAT,
                0,
                stride,
                std::ptr::null(),
            );
            (gl.glEnableVertexAttribArray)(uv_in);
            (gl.glVertexAttribPointer)(
                uv_in,
                2,
                GL_FLOAT,
                0,
                stride,
                (2 * float) as *const _,
            );

            // **Drained here and not per frame.** A GL error is sticky until
            // read, and everything that can raise one for these programs —
            // compiling, linking, binding a buffer, describing an attribute —
            // has just happened. Checking again on every `draw` would put a
            // call that some drivers make synchronous on the 30 fps path to
            // catch errors that cannot occur there.
            //
            // A non-zero code here means the program linked and is still
            // wrong, which is the case that otherwise shows up as a black
            // screen with nothing in the journal.
            let complaint = (gl.glGetError)();
            if complaint != 0 {
                return Err(format!(
                    "the {which:?} program linked and then GL reported 0x{complaint:04X} while it \
                     was being set up, so it would have drawn nothing and said nothing"
                ));
            }

            let at = |name: &std::ffi::CStr| (gl.glGetUniformLocation)(program, name.as_ptr());
            Built {
                program,
                u_field: at(c"field"),
                u_levels: at(c"levels"),
                u_colour: at(c"line_colour"),
                u_half_width: at(c"half_width"),
                u_board: at(c"board"),
                u_grid: at(c"grid"),
                u_cell_px: at(c"cell_px"),
                u_full_radius: at(c"full_radius"),
                u_cell_colour: at(c"cell_colour"),
                u_cells: at(c"cells"),
                u_viewport: at(c"viewport"),
            }
        };
        self.built[which.slot()] = Some(built);
        Ok(built)
    }

    /// **Draw one frame into whatever framebuffer the host has bound.**
    ///
    /// There is no `glBindFramebuffer` here and that is the design: R-206 D1
    /// puts the host in charge of creating and binding the target, and a pack
    /// that bound one of its own would be drawing somewhere the host is not
    /// going to present. There is no `glClear` either — every one of these
    /// shaders writes every pixel of the surface, so clearing first would be
    /// 2 MPx of work thrown away.
    ///
    /// # Errors
    /// A shader that would not build, or a process with no GLES.
    pub(crate) fn draw(
        &mut self,
        uploaded: &Uploaded<'_>,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        let gl = gles()?;
        let built = self.build(gl, uploaded.program())?;
        #[allow(clippy::cast_possible_wrap)]
        let (w, h) = (width as i32, height as i32);
        #[allow(clippy::cast_precision_loss)]
        let (wf, hf) = (width as f32, height as f32);

        // SAFETY: the context is current and the target is bound — both are
        // `rivetos_addon::gl`'s contract for this call. Every object named is
        // this struct's, and every pointer is into a slice that outlives the
        // upload it is passed to.
        unsafe {
            (gl.glViewport)(0, 0, w, h);
            (gl.glUseProgram)(built.program);
            (gl.glActiveTexture)(GL_TEXTURE0);
            (gl.glBindTexture)(GL_TEXTURE_2D, self.texture);
            match *uploaded {
                Uploaded::Contours {
                    field,
                    width: fw,
                    height: fh,
                    levels,
                    half_width,
                    colour,
                } => {
                    // **`GL_LINEAR`, and it is doing real work.** The field is
                    // 96x54 stretched over 1920x1080; nearest sampling would
                    // draw 96 flat steps and `fwidth` would read zero inside
                    // each one, which is a screen of solid colour rather than
                    // a wrong-looking contour.
                    (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
                    (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
                    (gl.glTexImage2D)(
                        GL_TEXTURE_2D,
                        0,
                        GL_LUMINANCE as i32,
                        fw,
                        fh,
                        0,
                        GL_LUMINANCE,
                        GL_UNSIGNED_BYTE,
                        field.as_ptr().cast(),
                    );
                    (gl.glUniform1i)(built.u_field, 0);
                    (gl.glUniform1f)(built.u_levels, levels);
                    (gl.glUniform1f)(built.u_half_width, half_width);
                    (gl.glUniform3f)(built.u_colour, colour[0], colour[1], colour[2]);
                }
                Uploaded::Life {
                    board,
                    cols,
                    rows,
                    cell_px,
                    full_radius,
                    colour,
                } => {
                    // **`GL_NEAREST`, and this one is load-bearing the other
                    // way.** A cell is a cell; interpolating between two would
                    // bleed a dying cell's heat into its neighbour and turn
                    // the trail into a smear.
                    (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
                    (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
                    #[allow(clippy::cast_possible_truncation)]
                    (gl.glTexImage2D)(
                        GL_TEXTURE_2D,
                        0,
                        GL_LUMINANCE_ALPHA as i32,
                        cols as i32,
                        rows as i32,
                        0,
                        GL_LUMINANCE_ALPHA,
                        GL_UNSIGNED_BYTE,
                        board.as_ptr().cast(),
                    );
                    (gl.glUniform1i)(built.u_board, 0);
                    (gl.glUniform2f)(built.u_grid, cols, rows);
                    (gl.glUniform2f)(built.u_cell_px, cell_px.0, cell_px.1);
                    (gl.glUniform1f)(built.u_full_radius, full_radius);
                    (gl.glUniform3f)(built.u_cell_colour, colour[0], colour[1], colour[2]);
                }
                Uploaded::Boxes {
                    cells,
                    cols,
                    rows,
                    cell_px,
                } => {
                    (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
                    (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
                    #[allow(clippy::cast_possible_truncation)]
                    (gl.glTexImage2D)(
                        GL_TEXTURE_2D,
                        0,
                        GL_RGB as i32,
                        cols as i32,
                        rows as i32,
                        0,
                        GL_RGB,
                        GL_UNSIGNED_BYTE,
                        cells.as_ptr().cast(),
                    );
                    (gl.glUniform1i)(built.u_cells, 0);
                    (gl.glUniform2f)(built.u_grid, cols, rows);
                    (gl.glUniform2f)(built.u_cell_px, cell_px.0, cell_px.1);
                    (gl.glUniform2f)(built.u_viewport, wf, hf);
                }
            }
            (gl.glDrawArrays)(GL_TRIANGLE_STRIP, 0, 4);
        }
        Ok(())
    }
}

/// Compile one shader, or hand back the driver's log.
///
/// # Safety
/// A context must be current.
unsafe fn compile(gl: &Gles, kind: u32, source: &str) -> Result<u32, String> {
    // SAFETY: forwarded from this function's own contract. `source` outlives
    // the `glShaderSource` call, which is the only thing that reads it.
    unsafe {
        let shader = (gl.glCreateShader)(kind);
        let pointer = source.as_ptr().cast::<c_char>();
        #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
        let length = source.len() as i32;
        (gl.glShaderSource)(shader, 1, &raw const pointer, &raw const length);
        (gl.glCompileShader)(shader);
        let mut ok: i32 = 0;
        (gl.glGetShaderiv)(shader, GL_COMPILE_STATUS, &raw mut ok);
        if ok == 0 {
            return Err(shader_log(gl, shader));
        }
        Ok(shader)
    }
}

/// Read one info log, whichever kind it is.
///
/// # Safety
/// A context must be current and `object` must be of the kind `get`/`read`
/// expect.
unsafe fn info_log(
    length: i32,
    read: impl FnOnce(i32, *mut i32, *mut c_char),
) -> String {
    if length <= 0 {
        return "the driver offered no log, which is itself worth reporting".to_string();
    }
    #[allow(clippy::cast_sign_loss)]
    let mut buffer = vec![0u8; length as usize];
    let mut written: i32 = 0;
    read(length, &raw mut written, buffer.as_mut_ptr().cast::<c_char>());
    #[allow(clippy::cast_sign_loss)]
    buffer.truncate(written.max(0) as usize);
    String::from_utf8_lossy(&buffer).trim_end().to_string()
}

/// # Safety
/// A context must be current and `shader` must be a shader name.
unsafe fn shader_log(gl: &Gles, shader: u32) -> String {
    // SAFETY: forwarded from this function's own contract.
    unsafe {
        let mut length: i32 = 0;
        (gl.glGetShaderiv)(shader, GL_INFO_LOG_LENGTH, &raw mut length);
        info_log(length, |max, written, into| {
            (gl.glGetShaderInfoLog)(shader, max, written, into);
        })
    }
}

/// # Safety
/// A context must be current and `program` must be a program name.
unsafe fn program_log(gl: &Gles, program: u32) -> String {
    // SAFETY: forwarded from this function's own contract.
    unsafe {
        let mut length: i32 = 0;
        (gl.glGetProgramiv)(program, GL_INFO_LOG_LENGTH, &raw mut length);
        info_log(length, |max, written, into| {
            (gl.glGetProgramInfoLog)(program, max, written, into);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Every shader is valid ES 2.0 by inspection of the things a test can
    /// inspect without a driver**, which is not much — but it is exactly the
    /// set of mistakes that have been made in this file's ancestor: a missing
    /// `precision` qualifier (a link error on the V3D and not on desktop), a
    /// name in the Rust side that the GLSL side does not declare, and a
    /// `#extension` line anywhere but first.
    #[test]
    fn the_shaders_declare_what_the_rust_side_looks_up() {
        for (program, src) in [
            (Program::Contours, CONTOURS_FRAGMENT_SRC),
            (Program::Life, LIFE_FRAGMENT_SRC),
            (Program::Boxes, BOXES_FRAGMENT_SRC),
        ] {
            assert!(
                src.contains("precision highp float;"),
                "{program:?} has no precision qualifier — it links on desktop and not on the V3D"
            );
            assert!(src.ends_with("}\n"), "{program:?} does not end in a closed main");
            if let Some(rest) = src.strip_prefix("#extension") {
                assert!(
                    rest.contains(": require\n"),
                    "{program:?} has an #extension line that is not a requirement"
                );
            } else {
                assert!(
                    !src.contains("#extension"),
                    "{program:?} has an #extension line that is not first, which ES 2.0 refuses"
                );
            }
            assert!(
                src.contains("gl_FragColor"),
                "{program:?} writes no fragment colour"
            );
        }
    }

    /// The vertex shader declares the two attributes [`Gpu::build`] looks up
    /// by name, and every fragment shader consumes the varying it writes.
    #[test]
    fn the_vertex_shader_and_the_fragment_shaders_agree_on_their_varying() {
        assert!(VERTEX_SRC.contains("attribute vec2 position;"));
        assert!(VERTEX_SRC.contains("attribute vec2 uv_in;"));
        assert!(VERTEX_SRC.contains("varying vec2 uv;"));
        for src in [CONTOURS_FRAGMENT_SRC, LIFE_FRAGMENT_SRC, BOXES_FRAGMENT_SRC] {
            assert!(src.contains("varying vec2 uv;"), "a fragment shader drops the varying");
        }
    }

    /// **Alpha is 1.0 in every shader**, because the background is the opaque
    /// parent surface — the widgets are a separate subsurface over it
    /// (`rivet_sdk::gpu::take_overlay`). A background that wrote anything else
    /// would make the compositor blend it against whatever is behind, which on
    /// the bottom layer is nothing.
    #[test]
    fn every_shader_writes_an_opaque_pixel() {
        for src in [CONTOURS_FRAGMENT_SRC, LIFE_FRAGMENT_SRC, BOXES_FRAGMENT_SRC] {
            assert!(
                src.contains(", 1.0);\n"),
                "a shader writes a fragment whose alpha is not 1.0"
            );
        }
    }

    /// The three programs occupy three distinct slots, which is what makes
    /// [`Gpu::built`] a cache rather than a source of cross-talk.
    #[test]
    fn the_programs_do_not_share_a_cache_slot() {
        let slots = [
            Program::Contours.slot(),
            Program::Life.slot(),
            Program::Boxes.slot(),
        ];
        assert_eq!(slots, [0, 1, 2]);
        let mut seen = slots.to_vec();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 3, "two programs share a slot");
    }

    /// **The catalogue's claim, checked against the code that has to honour
    /// it.**
    ///
    /// `Drawn::Gpu` and `Drawn::Either` are a promise a host acts on: it binds
    /// a framebuffer and asks. If `uploaded` then answers `None` the host gets
    /// a refusal it cannot distinguish from a broken driver, and the pack is
    /// the thing that lied.
    ///
    /// This walks every id the catalogue describes, builds the background, and
    /// requires the two to agree **in both directions** — a background with a
    /// GPU path that forgot to say so is a wasted rasteriser, and one that says
    /// so without having it is the failure above.
    ///
    /// Needs no driver: `uploaded` packs bytes and touches no GL.
    #[test]
    fn the_catalogue_does_not_promise_a_path_that_is_not_there() {
        use crate::background::Screensaver;
        use crate::rng::Mt19937;

        for described in crate::catalogue::describe() {
            for theme in &described.themes {
                let which = crate::catalogue::resolve(&described.id, &theme.id)
                    .expect("the catalogue describes what it resolves");
                let saver = Screensaver::new_with_rng(which, 320, 180, Mt19937::new(7));
                let mut scratch = Vec::new();
                let has_path = saver.uploaded(&mut scratch).is_some();
                assert_eq!(
                    has_path,
                    described.drawn.on_gpu(),
                    "{}/{}: the catalogue says on_gpu()={} and the code says {has_path}",
                    described.id,
                    theme.id,
                    described.drawn.on_gpu()
                );
                // Every background still draws on the CPU, which is R-206 D3
                // and the reason none of the three is `Drawn::Gpu`.
                assert!(
                    described.drawn.on_cpu(),
                    "{}: no background in this pack may drop its CPU path",
                    described.id
                );
            }
        }
    }

    /// **The bytes a background hands over are the size the upload claims.**
    ///
    /// `glTexImage2D` is told a width, a height and a format, and it reads
    /// `width * height * channels` bytes from the pointer. A slice shorter
    /// than that is the one way this module can still read past the end of
    /// something — the boundary hands over integers, but the texture upload
    /// is the pack's own pointer and the pack's own arithmetic.
    #[test]
    fn every_upload_is_as_long_as_the_texture_it_describes() {
        use crate::background::Screensaver;
        use crate::rng::Mt19937;

        for described in crate::catalogue::describe() {
            if !described.drawn.on_gpu() {
                continue;
            }
            for theme in &described.themes {
                let which = crate::catalogue::resolve(&described.id, &theme.id)
                    .expect("described ids resolve");
                let saver = Screensaver::new_with_rng(which, 1920, 1080, Mt19937::new(11));
                let mut scratch = Vec::new();
                let uploaded = saver.uploaded(&mut scratch).expect("claims a GPU path");
                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                let (bytes, want) = match uploaded {
                    Uploaded::Contours {
                        field,
                        width,
                        height,
                        ..
                    } => (field.len(), (width * height) as usize),
                    Uploaded::Life {
                        board, cols, rows, ..
                    } => (board.len(), (cols * rows) as usize * 2),
                    Uploaded::Boxes {
                        cells, cols, rows, ..
                    } => (cells.len(), (cols * rows) as usize * 3),
                };
                assert_eq!(
                    bytes, want,
                    "{}/{}: the upload is {bytes} bytes and the texture wants {want}",
                    described.id, theme.id
                );
            }
        }
    }

    /// Without a driver, construction fails with the resolver's sentence
    /// rather than panicking — the case every test host is in.
    #[test]
    fn with_no_driver_the_gpu_declines_to_exist() {
        match Gpu::new() {
            Ok(_) => { /* a host that really has GLES linked in; fine. */ }
            Err(why) => assert!(
                why.starts_with("gl"),
                "the failure should name the missing symbol: {why}"
            ),
        }
    }
}
