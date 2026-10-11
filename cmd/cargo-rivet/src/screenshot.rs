//! `cargo rivet screenshot` — **a capture off a board, as a PNG on your disk.**
//!
//! `PLAN.md` §104.39, ruled 2026-09-01: the capture may be full-resolution QOI,
//! and what the workstation software hands the maintainer is a PNG
//! (.rulings/ R-326). The board writes QOI because an encoder is sixty lines
//! where a compressor is a dependency (`rivet-sdk/src/qoi.rs` argues it); **the
//! conversion was always the workstation's half of that split, and until this
//! verb the workstation did not do it.** A capture taken by an operator or an
//! agent arrived as 67 KB of base64 inside a `get file` answer and there was
//! nothing anywhere in this repository that could turn it back into a picture.
//!
//! # It converts. It does not press the button — and that is a decision
//!
//! `ADR-012` §9.3 makes `apply input capture` an action an agent may send, so
//! this verb *could* take the picture before fetching it. It deliberately does
//! not, and the reason is not tidiness:
//!
//! 1. **The launcher refuses to capture while an application holds the
//!    display**, correctly — it stops drawing when it hands the screen over, so
//!    the frame it still holds is the one from BEFORE that (`PLAN.md` §108.5).
//!    **That refusal goes to the launcher's own log, not to the control plane.**
//!    `apply input` succeeds, because the press *was* delivered. A `--take`
//!    would therefore report success, fetch the file that was already there, and
//!    hand back a real screenshot of a real moment that is not the moment asked
//!    about. That failure has already fooled the agent who wrote the lease
//!    mechanism, twice, in one afternoon; a verb that reproduced it on demand
//!    would be worse than one that only converts.
//! 2. **There is no freshness signal a workstation can trust.** The launcher
//!    writes the file after the press returns, and `modified_ms` is the
//!    *board's* clock — on a Pi with no RTC that is not comparable with this
//!    host's (`get file`'s own field description says to treat it with
//!    suspicion). So this end cannot tell a new capture from the previous one.
//! 3. **An application takes its own picture, into its own data directory.** The
//!    path differs per application, so a verb that pressed would also have to
//!    guess which file the press produced.
//!
//! What it does instead is **report the frame's own timestamp and print the
//! command that takes a new one**, so the operator decides. `AGENTS.md` §5:
//! *"a wrong guess does not fail — it succeeds"*.
//!
//! # The chain, and where each link lives
//!
//! | Step | Where |
//! | :-- | :-- |
//! | `get file <path>` over `ADR-035`'s control plane | `rivet_remote::command`, `rivet_remote::endpoint` — the same two calls `cargo rivet run` makes |
//! | reading the answer | `rivet_core::json` |
//! | undoing `content` | `rivet_core::base64` — **one implementation, merged 2026-09-01**; this verb was going to be its third copy |
//! | QOI | [`crate::qoi`] |
//! | PNG | [`crate::png`] |
//!
//! Nothing in that table is spelled twice, and the base64 row is the one that
//! nearly was: `rivet-core` held an encoder in `operation/file.rs` and a decoder
//! in another module, and the two never mentioned each other.
//!
//! # `--holder`: the frame of whatever application is drawing
//!
//! An application holding the display saves its own frame into its own data
//! directory, under the name the SDK's `screen` module owns (`CAPTURE_FILE`
//! here). Which application that is, the machine says and this end does not
//! guess: `get status` carries `display_leased`, the live lease, and its own
//! field description forbids inferring it from `auto_run` or from which kiosk
//! application is started. So `--holder` asks `get status` first and composes
//! the path from the answer with the same two constants the launcher's default
//! is composed from.
//!
//! **Nobody holding the display is an outcome, not a failure.** The launcher is
//! drawing its own menu, or the machine is headless; either way there is no
//! application's frame to fetch, and a caller polling for one is told so with a
//! successful exit and nothing written. The one silence that is NOT read as
//! "nobody" is a status answer that could not have said otherwise: a supervisor
//! older than the field omits it too, so absence counts only beside
//! `display_master` or `version`, which every supervisor that knows the field
//! also sends.
//!
//! **Still nothing is pressed.** `--holder` reads one more answer and fetches a
//! file that already exists; every reason above for not capturing holds
//! unchanged.
//!
//! `--json` prints the outcome as one object for a program to read. The human
//! report and the object are both rendered from `Outcome`, so they cannot
//! disagree about what happened (`ADR-006` §4).

use std::path::PathBuf;

use rivet_core::json::Json;
use rivet_core::service::ServiceName;
use rivet_remote::endpoint::{Endpoint, IDENTITY_AND_PIN};
use rivet_remote::tls::CallsTools;

/// The file `rivet_sdk::screen::Screen::capture` writes.
///
/// **A second spelling of a name that module owns**, and it is one because the
/// SDK's `screen` module is behind a feature that links `libdrm` and cannot be
/// depended on from a host tool. It is a `--path` default and nothing more: an
/// operator whose application writes somewhere else passes the path, and this
/// constant never decides anything on its own.
const CAPTURE_FILE: &str = "screen.qoi";

/// Where application `name`'s saved frame is: `/state/app/<name>/data/screen.qoi`.
///
/// **Composed rather than written out.** `rivet_core::service::data_dir` owns
/// the shape of an application's data directory; spelling it here would be a
/// second authority for a path the board derives (`ADR-011`). Taking a
/// [`ServiceName`] rather than a string is what keeps a name the machine sent
/// from carrying a `/` into the path.
fn capture_path_for(name: &ServiceName) -> String {
    use rivet_core::service::{BUILT_IN_VOLUME, data_dir};

    format!("{}/{CAPTURE_FILE}", data_dir(BUILT_IN_VOLUME, name))
}

/// Where the compositor's captures land:
/// `/state/app/rivet-displayd/data/screen.qoi`.
///
/// **It was the LAUNCHER's directory until 2026-09-20** — `ADR-058` §5 moved
/// it, and the move is a correction rather than a relocation. Under the lease
/// model a capture came from whoever held DRM master, which was the launcher,
/// so this tool only ever fetched **the launcher's own frame** and never a
/// lessee's. It looked like a picture of the screen and was a picture of one
/// program.
///
/// `rivet-displayd` is the first program on this machine that can see the
/// screen, so it is the one that writes the file.
///
/// **A board running an older image still works**: this is a `--path` default
/// and nothing more, so an operator pointed at a machine with no compositor
/// passes the launcher's path and gets exactly what they used to.
fn default_capture_path() -> String {
    match ServiceName::new(rivet_core::compositor::PROVIDER) {
        Some(name) => capture_path_for(&name),
        // Unreachable: `PROVIDER` is a valid service name and `service.rs`'s own
        // tests say so. Composed anyway rather than unwrapped, because this
        // crate warns on `expect` and a default path is not worth a panic.
        None => format!("{}/app/{CAPTURE_FILE}", rivet_core::service::BUILT_IN_VOLUME),
    }
}

pub fn usage() -> String {
    format!(
        "usage: cargo rivet screenshot --host HOST:PORT --identity DIR --pin HEX\n\
         \x20                             [--path PATH | --holder] [--out FILE.png]\n\
         \x20                             [--qoi FILE.qoi] [--json]\n\
         \n\
         Fetches a capture from a board over ADR-035's control plane and writes\n\
         it as a PNG you can open. A Rivet takes screenshots as QOI, because an\n\
         encoder is sixty lines on a device where a compressor is a dependency;\n\
         turning one back into a picture is this end's job and this is it.\n\
         \n\
         \x20 cargo rivet screenshot --host pi:7777 --identity ./id --pin <hex>\n\
         \n\
         THIS DOES NOT TAKE THE PICTURE. It converts the one that is there. To\n\
         take a fresh one:\n\
         \n\
         \x20 cargo rivet run --host H:{port} --identity DIR --pin HEX -- apply input capture\n\
         \n\
         THE COMPOSITOR TAKES IT, since ADR-058 §5 — rivet-displayd, which is\n\
         the one program that can see the whole screen. The press still arrives\n\
         at rivet-launcher, because that is what provides the input endpoint,\n\
         and the launcher FORWARDS it rather than performing it.\n\
         \n\
         ~~It REFUSES while an application holds the display.~~ There is no such\n\
         refusal any more, and its absence is the point: under the lease model a\n\
         capture came from whoever held DRM master, so this fetched THE\n\
         LAUNCHER'S OWN FRAME and never a lessee's. It looked like a picture of\n\
         the screen and was a picture of one program.\n\
         \n\
         On an OLDER board, with no compositor, all of the above still holds and\n\
         the file is the launcher's: pass --path for it, or --holder to find\n\
         whatever an application wrote into its own data directory (ADR-036).\n\
         \n\
         screenshot options:\n\
         \x20 --host HOST:PORT   REQUIRED. Where rivet-mcpd is listening.\n\
         {IDENTITY_AND_PIN}\
         \x20 --path PATH        the capture ON THE BOARD. Default:\n\
         \x20                    {default}\n\
         \x20 --holder           the frame of the APPLICATION HOLDING THE DISPLAY,\n\
         \x20                    whichever it is. Asks the machine's `get status`\n\
         \x20                    (display_leased) and fetches that application's\n\
         \x20                    {holder_path}.\n\
         \x20                    When no application holds the display, NOTHING is\n\
         \x20                    fetched, no file is written, and this exits 0 saying\n\
         \x20                    so. Refused together with --path: two answers to one\n\
         \x20                    question.\n\
         \x20 --json             print ONE JSON object on stdout, and nothing else\n\
         \x20                    there, for a program to read. Keys: outcome\n\
         \x20                    (fetched | no-application-holds-the-display),\n\
         \x20                    holder, board_path, frame_modified_ms (the board's\n\
         \x20                    clock), out (the PNG written). Each is null when it\n\
         \x20                    does not apply. A failure prints no JSON: it exits\n\
         \x20                    non-zero with the reason on stderr.\n\
         \x20 --out FILE.png     where the PNG goes. Default ./screen.png, and an\n\
         \x20                    existing file is replaced — a screenshot is taken\n\
         \x20                    again and again, unlike a key.\n\
         \x20 --qoi FILE.qoi     also keep the bytes exactly as the board sent\n\
         \x20                    them. Nothing needs this to view the picture; it\n\
         \x20                    is what to attach to a report about a decoder.\n\
         \n\
         THE PNG IS BIG AND THAT IS DELIBERATE. It is written with DEFLATE\n\
         stored blocks — no compressor, no dependency in the binary that holds\n\
         a signing key (ADR-009 §5) — so a 1920x1080 capture is about 6 MB. That\n\
         is acceptable for a debugging artefact and would not be for anything\n\
         this project served or stored.\n\
         \n\
         `rivet get file` is what this sends, so everything that operation\n\
         refuses it refuses here in the machine's own words — and its 1 MiB read\n\
         limit is the cap on how large a capture can be fetched at all.\n",
        port = rivet_config::control_plane::DEFAULT_PORT,
        default = default_capture_path(),
        holder_path = holder_path_pattern(),
    )
}

/// `/state/app/<holder>/data/screen.qoi`, for the help — composed the way
/// [`capture_path_for`] composes a real one, so the help cannot name a
/// different shape from the one fetched.
fn holder_path_pattern() -> String {
    const PLACEHOLDER: &str = "HOLDER";
    match ServiceName::new(PLACEHOLDER) {
        Some(name) => capture_path_for(&name).replace(PLACEHOLDER, "<holder>"),
        None => format!("<data directory>/{CAPTURE_FILE}"),
    }
}

/// Which frame on the machine.
#[derive(Debug, PartialEq, Eq)]
pub enum Frame {
    /// A file named outright, or the launcher's by default.
    Path(String),
    /// Whatever application `get status` says holds the display.
    Holder,
}

/// What `screenshot` was asked to do.
#[derive(Debug)]
pub struct ScreenshotArgs {
    pub endpoint: Endpoint,
    /// Which capture on the machine.
    pub frame: Frame,
    /// Where the PNG goes.
    pub out: PathBuf,
    /// Where the QOI bytes go, if they were asked for.
    pub qoi: Option<PathBuf>,
    /// One JSON object on stdout instead of the report.
    pub json: bool,
}

/// # Errors
///
/// A flag with no value, a flag this verb does not take, `--holder` with
/// `--path`, or either half of `ADR-025` §2's trust without the other.
pub fn parse(args: &[String]) -> Result<ScreenshotArgs, String> {
    let mut endpoint = Endpoint::default();
    let mut path: Option<String> = None;
    let mut out: Option<PathBuf> = None;
    let mut qoi: Option<PathBuf> = None;
    let mut holder = false;
    let mut json = false;

    let mut index = 0;
    while index < args.len() {
        let value = |flag: &str| {
            args.get(index + 1)
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match args[index].as_str() {
            "--path" => {
                path = Some(value("--path")?);
                index += 2;
            }
            "--out" => {
                out = Some(PathBuf::from(value("--out")?));
                index += 2;
            }
            "--qoi" => {
                qoi = Some(PathBuf::from(value("--qoi")?));
                index += 2;
            }
            "--holder" => {
                holder = true;
                index += 1;
            }
            "--json" => {
                json = true;
                index += 1;
            }
            // `--host`, `--identity` and `--pin`, in the one spelling every verb
            // that reaches a machine uses (`ADR-034` §11.5.1).
            _ => match endpoint.read(args, index)? {
                Some(next) => index = next,
                None => {
                    return Err(format!(
                        "{:?} is not an option `screenshot` takes.\n\n{}",
                        args[index],
                        usage()
                    ));
                }
            },
        }
    }
    endpoint.paired()?;

    let frame = match (holder, path) {
        (true, Some(path)) => {
            return Err(format!(
                "--holder and --path {path} were both given, and they are two answers to one \
                 question: --holder fetches whichever application's frame the machine says is \
                 on the display, --path fetches the file named. Give one."
            ));
        }
        (true, None) => Frame::Holder,
        (false, path) => Frame::Path(path.unwrap_or_else(default_capture_path)),
    };

    Ok(ScreenshotArgs {
        endpoint,
        frame,
        out: out.unwrap_or_else(|| PathBuf::from("screen.png")),
        qoi,
        json,
    })
}

/// Who holds the display, out of the JSON `rivet get status` answered with.
///
/// `Ok(None)` is the machine saying nobody does. See the module documentation
/// for why an absent `display_leased` counts as that only beside
/// `display_master` or `version`.
///
/// # Errors
///
/// Unreadable JSON; an answer about another subject; a `display_leased` that
/// is not a string or not a service name; or a silence that could come from a
/// supervisor that does not know the field.
fn read_holder(answer: &str) -> Result<Option<ServiceName>, String> {
    let json = rivet_core::json::parse(answer).map_err(|why| {
        format!(
            "the machine's status answer is not JSON this build can read ({why}). It said:\n{answer}"
        )
    })?;
    let subject = rivet_core::operation::status::SUBJECT;
    match json.get("subject").and_then(Json::as_str) {
        Some(found) if found == subject => {}
        found => {
            return Err(format!(
                "this asked the machine for its `{subject}` and the answer is about {found:?}. \
                 Nothing is fetched on the strength of it."
            ));
        }
    }

    match json.get("display_leased") {
        Some(Json::Str(name)) => ServiceName::new(name).map(Some).ok_or_else(|| {
            format!(
                "the machine says {name:?} holds the display, and that is not a service name \
                 ({}), so no path is composed from it.",
                ServiceName::RULE
            )
        }),
        Some(Json::Null) | None => {
            if json.get("display_master").is_some() || json.get("version").is_some() {
                Ok(None)
            } else {
                Err("the machine's status names no display holder, and it also carries neither \
                     `display_master` nor `version`, which every supervisor that reports a \
                     holder sends beside it. This machine's supervisor may be older than the \
                     field, so its silence is not read as \"nobody holds the display\". Nothing \
                     was fetched. `--path` names a frame outright."
                    .to_string())
            }
        }
        Some(other) => Err(format!(
            "the machine's `display_leased` is {other:?} and not a service name. Nothing is \
             fetched on the strength of it."
        )),
    }
}

/// What one `get file` answer says about a capture.
///
/// **A type rather than four loose values**, because [`read_answer`] is the only
/// part of this verb that can be tested without a board and the thing worth
/// testing is exactly this extraction. Every refusal it produces is about a JSON
/// document a machine composed, so each names what was found.
struct Capture {
    /// **Never printed**, for [`crate::qoi::Image`]'s reason — see this type's
    /// `Debug`.
    bytes: Vec<u8>,
    /// What the machine said the FILE weighs — not this window. The two are
    /// the same only for a capture that fits in one answer.
    size_bytes: i64,
    /// Whether the file ends where this window does. `true` from a machine
    /// that predates the window, which always answered whole files.
    complete: bool,
    /// Milliseconds since the epoch, **by the board's clock**. See the module
    /// documentation for why that is not comparable with this host's.
    modified_ms: Option<i64>,
}

/// **The counts, never the capture.** A derived `Debug` would print 67 KB of
/// pixels into a test failure and bury the sentence somebody needed.
impl std::fmt::Debug for Capture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Capture {{ {} bytes arrived, {} claimed, modified_ms {:?} }}",
            self.bytes.len(),
            self.size_bytes,
            self.modified_ms
        )
    }
}

/// The capture's bytes, out of the JSON `rivet get file` answered with.
///
/// # Why every one of these is a refusal rather than a fallback
///
/// `AGENTS.md` §5. This end is holding a document another machine composed, and
/// the useful thing to say when it is not what was expected is *what it actually
/// says* — not a picture assembled from whatever fields happened to parse. The
/// machine's own `content_omitted` sentence is carried through verbatim where
/// there is one, because it is more specific than anything this end could
/// compose about a file it has never seen.
///
/// # Errors
///
/// Unreadable JSON; an answer about another subject or another path; nothing at
/// the path; a path that is not a regular file; `content` omitted, with the
/// machine's reason; `content` that is text rather than base64; base64 that does
/// not decode; or a length that disagrees with what the machine counted.
fn read_answer(answer: &str, asked_for: &str) -> Result<Capture, String> {
    let json = rivet_core::json::parse(answer).map_err(|why| {
        format!(
            "the machine's answer is not JSON this build can read ({why}). It said:\n{answer}"
        )
    })?;

    let string = |key: &str| json.get(key).and_then(Json::as_str);
    let integer = |key: &str| json.get(key).and_then(Json::as_integer);

    // **The answer is checked to be about the question.** One connection can
    // carry several calls, and an answer bound to the wrong one would produce a
    // picture of the wrong file with nothing saying so.
    match string("path") {
        Some(path) if path == asked_for => {}
        Some(path) => {
            return Err(format!(
                "this asked about {asked_for} and the machine answered about {path}. That is a \
                 different file and nothing is decoded from it."
            ));
        }
        None => {
            return Err(format!(
                "the machine's answer carries no `path`, so this build cannot tell what it is \
                 about. It said:\n{answer}"
            ));
        }
    }

    if json.get("present") != Some(&Json::Bool(true)) {
        return Err(format!(
            "there is nothing at {asked_for} on that machine.\n\
             \x20 if   this is the launcher's capture, nothing has taken one since the \
             application's data directory was last emptied. `apply input capture` takes one — \
             see `cargo rivet screenshot --help` for what that will and will not photograph.\n\
             \x20 or   an application holding the display writes its own capture into its own \
             data directory, and --path is how to name it."
        ));
    }
    match string("type") {
        Some("file") => {}
        Some(other) => {
            return Err(format!(
                "{asked_for} is a {other} on that machine and a capture is a regular file. \
                 Nothing is decoded from it."
            ));
        }
        None => return Err(format!("the machine said nothing is at {asked_for}.")),
    }

    let Some(content) = string("content") else {
        let why = string("content_omitted").unwrap_or(
            "the machine gave no reason, which is itself worth reporting: `get file` promises \
             `content_omitted` whenever `content` is absent",
        );
        return Err(format!(
            "the machine read {asked_for} and would not give its contents:\n\
             \x20 {why}"
        ));
    };
    match string("content_encoding") {
        Some("base64") => {}
        // A QOI file is not valid UTF-8 — its magic is `qoif` but a scanout
        // buffer's bytes are not — so `text` here means the file at that path is
        // not a capture at all.
        Some(other) => {
            return Err(format!(
                "the machine says {asked_for} is `{other}` and not base64, which means it holds \
                 valid UTF-8. A QOI capture is binary and never reads as text, so whatever is at \
                 that path is not one."
            ));
        }
        None => {
            return Err(
                "the machine gave `content` and no `content_encoding`, and this build will not \
                 guess which of the two it is holding."
                    .to_string(),
            );
        }
    }

    let bytes = rivet_core::base64::decode(content).ok_or_else(|| {
        format!(
            "the machine said `content` is base64 and it does not decode as RFC 4648 §4. \
             {} characters arrived. Nothing is decoded from it — a lenient reading here would \
             produce bytes that look exactly like the right ones.",
            content.len(),
        )
    })?;

    // **The machine counted the file; this counts what arrived.** Two layers of
    // encoding stand between the two numbers, and a mistake in either produces a
    // picture rather than a failure. `rivet_remote`'s document reader makes the
    // same comparison for the same reason.
    let size_bytes = integer("size_bytes").ok_or_else(|| {
        format!("the machine's answer carries no `size_bytes` for {asked_for}, so what arrived \
                 cannot be checked against what it read.")
    })?;

    // **What this window CLAIMS to be, against what arrived.** Until
    // 2026-09-21 the comparison was against `size_bytes` — the whole file —
    // because an answer was always the whole file. A windowed answer says how
    // much of the file it covers in `content_bytes`, and that is the number
    // this end can check. `read_windows` checks the assembled total against
    // `size_bytes` afterwards, so both comparisons still happen.
    //
    // **An answer with neither field is one from a machine that predates the
    // window**, and it is read the old way rather than refused: it carried the
    // whole file or nothing.
    let complete = match json.get("content_complete") {
        Some(Json::Bool(said)) => *said,
        _ => true,
    };
    let claimed = integer("content_bytes").unwrap_or(size_bytes);
    if i64::try_from(bytes.len()).unwrap_or(i64::MAX) != claimed {
        return Err(format!(
            "the machine read {claimed} bytes from {asked_for} and {} arrived. Something \
             between the two ends lost or added bytes; nothing is decoded from a capture that \
             is not the file.",
            bytes.len(),
        ));
    }

    Ok(Capture {
        bytes,
        size_bytes,
        complete,
        modified_ms: integer("modified_ms"),
    })
}

/// `screenshot`, with a real connection behind it.
///
/// # Errors
///
/// This end's refusals above, a missing or unusable transport, the machine's own
/// refusal, a capture that is not readable QOI, or a file that cannot be written.
pub fn execute(args: &[String]) -> Result<String, String> {
    let parsed = parse(args)?;

    // Composed BEFORE a key is read or a socket is opened, which is the order
    // `cargo rivet run` uses: a command this end cannot send costs nothing to
    // refuse and a connection costs a process on the board. With `--holder` the
    // file's path is not known until the machine has answered, so the status
    // call is the one composed here.
    match &parsed.frame {
        Frame::Path(path) => rivet_remote::command::translate(&["get", "file", path])?,
        Frame::Holder => rivet_remote::command::translate(&["get", "status"])?,
    };

    let transport = parsed.endpoint.open()?.ok_or_else(|| {
        format!(
            "--identity and --pin are required: `screenshot` speaks ADR-035's control plane and \
             has no other transport.\n\n{}",
            usage()
        )
    })?;
    let outcome = fetch(&transport, &parsed)?;
    Ok(if parsed.json {
        format!("{}\n", outcome_json(&outcome))
    } else {
        report(&parsed, &outcome)
    })
}

/// What one run of this verb came to. Both renderings read this and nothing
/// else.
#[derive(Debug)]
enum Outcome {
    /// A frame was fetched, converted and written.
    Fetched(Fetched),
    /// `--holder` was asked and the machine says no application holds the
    /// display. Nothing was fetched and nothing was written.
    NoHolder,
}

/// A frame that was fetched and written.
#[derive(Debug)]
struct Fetched {
    /// The application `get status` named, when `--holder` asked it.
    holder: Option<String>,
    /// The file's path on the machine.
    board_path: String,
    /// Milliseconds since the epoch by the board's clock, as `get file` said.
    frame_modified_ms: Option<i64>,
    size_bytes: i64,
    width: u32,
    height: u32,
    channels: u8,
    opaque: bool,
    /// Where the PNG was written, and how large it is.
    out: PathBuf,
    png_bytes: usize,
    /// Where the QOI bytes were kept, if they were asked for.
    qoi: Option<PathBuf>,
}

/// **How many `get file` calls one capture may take before this gives up.**
///
/// A 1080p QOI is at most about eight megabytes — QOI never exceeds the raw
/// pixels by more than its header and a worst-case run — and one answer
/// carries a mebibyte, so sixteen is roughly double what the largest possible
/// capture needs. The bound exists because `content_complete` comes from the
/// machine: a board answering `false` for ever would otherwise be an infinite
/// loop in a tool an operator is watching.
const MOST_WINDOWS: usize = 16;

/// **Fetch a capture of any size, a window at a time.**
///
/// # Why this is a loop and was not one until 2026-09-21
///
/// `get file` read a file whole or refused it, and refused anything over a
/// mebibyte. A 1080p capture is usually smaller and **sometimes is not**:
/// `rivet-01` drawing Contours produced a 1,090,017-byte QOI — 41 KB over —
/// and this verb, whose entire job is to fetch that file, failed. Not for a
/// broken screen; for an interesting one. *"Screenshots work unless the
/// picture is busy"* is a broken tool, so `get file` grew a byte offset and
/// this grew the loop that uses it.
///
/// The pieces are concatenated as BYTES and decoded once, which is also why
/// `read_answer` tolerates a window that splits a UTF-8 character.
///
/// # Errors
///
/// Everything [`read_answer`] refuses, plus a machine whose windows do not
/// advance and one that never says it is complete.
/// **One stable snapshot, or a named reason it might not be one — B11.**
///
/// Nothing before this made the several `get file` calls one capture takes
/// add up to ONE version of the file. A capture that is overwritten between
/// two of those calls — `rivet-displayd` publishes a new one by rename,
/// which is atomic per call but says nothing about two SEPARATE calls landing
/// on either side of it — could be assembled from bytes that belong to two
/// different frames, and a same-size overwrite passed the one check this
/// function used to make (the assembled total against the final window's
/// own `size_bytes`) with nothing to say the picture was wrong.
///
/// So every window after the first is checked against the first window's own
/// `size_bytes` and, where both sides have one, `modified_ms`. A mismatch is
/// not a refusal from here: it is reported as `Ok(Err(reason))`, which
/// [`read_windows`] retries once, since a frame that is simply mid-change is
/// a resource this machine will most likely finish changing.
///
/// # Same-timestamp limitation
///
/// Two DIFFERENT captures that happen to share both a byte count and a
/// `modified_ms` — plausible for a static picture recaptured within the same
/// millisecond — are indistinguishable by this check, and a splice between
/// them would pass it. The wire protocol carries no per-capture identity
/// beyond these two fields; adding one is a format change and outside this
/// repair (`AGENTS.md` §1.1).
///
/// # Errors
///
/// Everything [`read_answer`] refuses, plus a machine whose windows do not
/// advance and one that never says it is complete.
fn read_windows_once(
    machine: &impl CallsTools,
    board_path: &str,
) -> Result<Result<Capture, String>, String> {
    let mut at: i64 = 0;
    let mut whole: Vec<u8> = Vec::new();
    // The first window's own `(size_bytes, modified_ms)`, to check every
    // later window against. `None` until window 0 answers.
    let mut first: Option<(i64, Option<i64>)> = None;

    for window in 0..MOST_WINDOWS {
        let offset = at.to_string();
        // The first call names no offset, so a machine that predates the
        // window still answers it — and answers completely, for every capture
        // small enough that it could have before.
        let words: Vec<&str> = if window == 0 {
            vec!["get", "file", board_path]
        } else {
            vec!["get", "file", board_path, offset.as_str()]
        };
        let call = rivet_remote::command::translate(&words)?;
        let answer = machine.call_tool(&call.tool, &call.arguments)?;
        let piece = read_answer(&answer, board_path)?;

        match first {
            None => first = Some((piece.size_bytes, piece.modified_ms)),
            Some((first_size, first_modified)) => {
                if piece.size_bytes != first_size {
                    return Ok(Err(format!(
                        "{board_path} was {first_size} bytes at the first window and \
                         {} bytes at window {window} of the same read",
                        piece.size_bytes
                    )));
                }
                // **Checked only when BOTH windows have one.** An older
                // machine that never sends `modified_ms` must read exactly
                // as it did before this check existed.
                if let (Some(a), Some(b)) = (first_modified, piece.modified_ms)
                    && a != b
                {
                    return Ok(Err(format!(
                        "{board_path} was modified_ms {a} at the first window and {b} at \
                         window {window} of the same read"
                    )));
                }
            }
        }

        if piece.bytes.is_empty() && !piece.complete {
            return Err(format!(
                "the machine answered {board_path} with no bytes at offset {at} and did not \
                 say the file was complete, so asking again would not make progress"
            ));
        }
        at += i64::try_from(piece.bytes.len()).unwrap_or(i64::MAX);
        whole.extend_from_slice(&piece.bytes);
        if piece.complete {
            let mut capture = piece;
            // **The assembled total against what the machine said the FILE
            // weighs.** `read_answer` checked each window against its own
            // `content_bytes`; this is the other half, and it is what catches
            // a window that was skipped or counted twice.
            if at != capture.size_bytes {
                return Err(format!(
                    "the machine says {board_path} holds {} bytes and {at} arrived across \
                     {} read(s). Nothing is decoded from a capture that is not the file.",
                    capture.size_bytes,
                    window + 1,
                ));
            }
            capture.bytes = whole;
            return Ok(Ok(capture));
        }
    }

    Err(format!(
        "{board_path} did not end within {MOST_WINDOWS} reads ({at} bytes so far). That is \
         more than the largest capture this build expects, so it is reported rather than \
         asked for again."
    ))
}

/// **Fetch a capture of any size, a window at a time — never a mix of two.**
///
/// # Why this is a loop and was not one until 2026-09-21
///
/// `get file` read a file whole or refused it, and refused anything over a
/// mebibyte. A 1080p capture is usually smaller and **sometimes is not**:
/// `rivet-01` drawing Contours produced a 1,090,017-byte QOI — 41 KB over —
/// and this verb, whose entire job is to fetch that file, failed. Not for a
/// broken screen; for an interesting one. *"Screenshots work unless the
/// picture is busy"* is a broken tool, so `get file` grew a byte offset and
/// this grew the loop that uses it.
///
/// The pieces are concatenated as BYTES and decoded once, which is also why
/// `read_answer` tolerates a window that splits a UTF-8 character.
///
/// # One retry, never a mixed result — B11
///
/// [`read_windows_once`] does the reading and reports whether the file's own
/// metadata stayed the same across every window of that attempt. A changed
/// answer is not decoded and is not returned: this calls it a second time,
/// since a frame caught mid-change once has most likely finished changing by
/// the next attempt, and refuses outright only if the SECOND attempt is
/// inconsistent too.
///
/// # Errors
///
/// Everything [`read_windows_once`] refuses outright, plus two consecutive
/// inconsistent reads of `board_path`.
fn read_windows(machine: &impl CallsTools, board_path: &str) -> Result<Capture, String> {
    match read_windows_once(machine, board_path)? {
        Ok(capture) => Ok(capture),
        Err(first_reason) => match read_windows_once(machine, board_path)? {
            Ok(capture) => Ok(capture),
            Err(second_reason) => Err(format!(
                "{board_path} changed while it was being read, twice in a row: {first_reason}; \
                 then, on a second attempt, {second_reason}. Nothing is decoded from a capture \
                 that might be two different frames spliced together."
            )),
        },
    }
}

/// **The verb, against any machine that answers tool calls.**
///
/// Split from [`execute`] so a test can hand it a fake machine: the no-holder
/// outcome's whole promise is about which calls are NOT made and which files
/// are NOT written, and only a caller that sees every call can check that.
///
/// # Errors
///
/// [`read_holder`]'s and [`read_answer`]'s refusals, the machine's own refusal,
/// a capture that is not readable QOI, or a file that cannot be written.
fn fetch(machine: &impl CallsTools, args: &ScreenshotArgs) -> Result<Outcome, String> {
    let (holder, board_path) = match &args.frame {
        Frame::Path(path) => (None, path.clone()),
        Frame::Holder => {
            let call = rivet_remote::command::translate(&["get", "status"])?;
            let answer = machine.call_tool(&call.tool, &call.arguments)?;
            match read_holder(&answer)? {
                None => return Ok(Outcome::NoHolder),
                Some(name) => (Some(name.as_str().to_owned()), capture_path_for(&name)),
            }
        }
    };

    let capture = read_windows(machine, &board_path).map_err(|why| match &holder {
        Some(name) => format!(
            "the machine says {name} holds the display, and its frame could not be fetched: {why}"
        ),
        None => why,
    })?;

    if let Some(path) = &args.qoi {
        std::fs::write(path, &capture.bytes)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }

    let image = crate::qoi::decode(&capture.bytes).map_err(|why| {
        format!("{board_path} is on that machine and is not QOI this build can read: {why}")
    })?;
    let opaque = image.is_opaque();
    let png = crate::png::encode(image.width, image.height, &image.pixels, !opaque);
    std::fs::write(&args.out, &png)
        .map_err(|e| format!("cannot write {}: {e}", args.out.display()))?;

    Ok(Outcome::Fetched(Fetched {
        holder,
        board_path,
        frame_modified_ms: capture.modified_ms,
        size_bytes: capture.size_bytes,
        width: image.width,
        height: image.height,
        channels: image.channels,
        opaque,
        out: args.out.clone(),
        png_bytes: png.len(),
        qoi: args.qoi.clone(),
    }))
}

/// The `outcome` words `--json` prints.
const FETCHED: &str = "fetched";
const NO_HOLDER: &str = "no-application-holds-the-display";

/// **One JSON object, on one line**, for a program to read.
///
/// Every key is present in every outcome and is `null` where it does not apply,
/// so a reader checks one shape rather than two. Strings go through
/// `rivet_core::journal::write_json_string`, the workspace's one JSON string
/// escaper.
fn outcome_json(outcome: &Outcome) -> String {
    fn string(value: Option<&str>) -> String {
        match value {
            Some(text) => {
                let mut out = String::new();
                rivet_core::journal::write_json_string(text, &mut out);
                out
            }
            None => "null".to_string(),
        }
    }

    let (word, holder, board_path, modified, out) = match outcome {
        Outcome::Fetched(fetched) => (
            FETCHED,
            fetched.holder.as_deref(),
            Some(fetched.board_path.as_str()),
            fetched.frame_modified_ms,
            Some(fetched.out.display().to_string()),
        ),
        Outcome::NoHolder => (NO_HOLDER, None, None, None, None),
    };
    format!(
        "{{\"outcome\":{},\"holder\":{},\"board_path\":{},\"frame_modified_ms\":{},\"out\":{}}}",
        string(Some(word)),
        string(holder),
        string(board_path),
        modified.map_or_else(|| "null".to_string(), |ms| ms.to_string()),
        string(out.as_deref()),
    )
}

/// What this verb says when it worked.
///
/// Separated from [`execute`] so it can be checked without a board — and worth
/// checking, because two of its lines exist to stop somebody believing something
/// untrue: the timestamp is the *board's* and the picture is of whatever the
/// launcher last drew.
fn report(args: &ScreenshotArgs, outcome: &Outcome) -> String {
    let host = args.endpoint.host.as_deref().unwrap_or("the machine");
    let fetched = match outcome {
        Outcome::NoHolder => {
            return format!(
                "holder   none. {host} says no application holds the display.\n\
                 \n\
                 NOTHING WAS FETCHED AND NO FILE WAS WRITTEN. The launcher is drawing its own\n\
                 screen, or the machine is headless. The launcher's last capture is fetched by\n\
                 running this without --holder.\n"
            );
        }
        Outcome::Fetched(fetched) => fetched,
    };

    let mut out = String::new();
    if let Some(name) = &fetched.holder {
        out.push_str(&format!(
            "holder   {name} holds the display, by the machine's get status\n"
        ));
    }
    out.push_str(&format!(
        "from     {} on {host}\n\
         size     {} bytes of QOI, {}×{}, {} channels declared\n",
        fetched.board_path, fetched.size_bytes, fetched.width, fetched.height, fetched.channels,
    ));
    if let Some(path) = &fetched.qoi {
        out.push_str(&format!("kept     {} (the bytes as sent)\n", path.display()));
    }
    out.push_str(&format!(
        "wrote    {} ({} bytes, {})\n",
        fetched.out.display(),
        fetched.png_bytes,
        if fetched.opaque {
            "8-bit RGB"
        } else {
            "8-bit RGBA — the capture is not fully opaque"
        },
    ));
    if let Some(ms) = fetched.frame_modified_ms {
        out.push_str(&format!(
            "taken    {ms} ms after the epoch, BY THE BOARD'S CLOCK. A Pi has no RTC, so that \
             is not comparable\n\
             \x20        with this host's time until something has corrected it (ADR-021). It \
             is here so a capture\n\
             \x20        that has not changed between two runs can be recognised.\n",
        ));
    }
    out.push_str(
        "\n\
         THIS IS THE FRAME THAT WAS ALREADY ON THAT MACHINE. Nothing was pressed.\n\
         `-- apply input capture` through `cargo rivet run` takes a new one, and since\n\
         ADR-058 §5 the COMPOSITOR takes it — so it is the whole screen rather than\n\
         one program's idea of it, whatever is on top. On an older board with no\n\
         compositor, an application holding the display writes its own picture and\n\
         `--holder` fetches that one.\n",
    );
    out
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn parse_of(list: &[&str]) -> Result<ScreenshotArgs, String> {
        let owned: Vec<String> = list.iter().map(|w| (*w).to_string()).collect();
        parse(&owned)
    }

    fn trust() -> Vec<&'static str> {
        vec!["--host", "pi:7777", "--identity", "./id", "--pin", "ab"]
    }

    #[test]
    fn the_defaults_are_the_compositors_capture_and_a_png_beside_you() {
        let parsed = parse_of(&trust()).expect("valid");
        assert_eq!(
            parsed.frame,
            Frame::Path("/state/app/rivet-displayd/data/screen.qoi".to_string())
        );
        assert_eq!(parsed.out, PathBuf::from("screen.png"));
        assert!(parsed.qoi.is_none());
        assert!(!parsed.json, "--json is asked for, never assumed");
    }

    // -- --holder and --json, as parsed -----------------------------------

    #[test]
    fn holder_asks_the_machine_and_names_no_path() {
        let mut words = trust();
        words.push("--holder");
        let parsed = parse_of(&words).expect("valid");
        assert_eq!(parsed.frame, Frame::Holder);
        assert!(!parsed.json);
    }

    #[test]
    fn json_is_a_switch_that_takes_no_value_and_combines_with_holder() {
        let mut words = trust();
        words.extend(["--json", "--holder", "--out", "x.png"]);
        let parsed = parse_of(&words).expect("valid");
        assert!(parsed.json);
        assert_eq!(parsed.frame, Frame::Holder);
        assert_eq!(parsed.out, PathBuf::from("x.png"), "--json consumed no value");
    }

    /// **`--holder` with `--path` is refused by name, in either order.** They
    /// are two answers to one question, and quietly preferring one would hand
    /// back a frame of a file the caller did not ask for.
    #[test]
    fn holder_with_path_is_refused_by_name_in_either_order() {
        for words in [
            ["--holder", "--path", "/state/app/x/data/screen.qoi"],
            ["--path", "/state/app/x/data/screen.qoi", "--holder"],
        ] {
            let mut all = trust();
            all.extend(words);
            let refused = parse_of(&all).expect_err("two answers to one question");
            assert!(refused.contains("--holder"), "{refused}");
            assert!(refused.contains("--path"), "{refused}");
        }
    }

    /// **A holder's frame path is composed from its name** by the same
    /// function the launcher's default goes through, and written out literally
    /// here so a change to `data_dir` or `CAPTURE_FILE` is seen.
    #[test]
    fn a_holders_frame_path_is_its_data_directorys_screen_qoi() {
        let name = ServiceName::new("aura-renderer").expect("a service name");
        assert_eq!(capture_path_for(&name), "/state/app/aura-renderer/data/screen.qoi");
        assert!(usage().contains("/state/app/<holder>/data/screen.qoi"), "{}", usage());
    }

    /// **Fleet Monitor proves the flags exist by reading this help**, so each
    /// flag starts a line of its own in the option list.
    #[test]
    fn the_help_names_holder_and_json_on_lines_of_their_own() {
        let usage = usage();
        for flag in ["--holder", "--json"] {
            assert!(
                usage.lines().any(|line| line.trim_start().starts_with(flag)),
                "{flag} starts no line of the help:\n{usage}"
            );
        }
        assert!(usage.contains(NO_HOLDER), "{usage}");
    }

    // -- who holds the display, from get status ---------------------------

    #[test]
    fn a_leased_display_names_its_holder() {
        let answer = r#"{"subject":"status","phase":"running","version":"0.1.86","health":"ok","mode":"kiosk","locked":false,"display_master":"rivet-launcher","display_leased":"aura-renderer","services":[]}"#;
        let holder = read_holder(answer).expect("readable").expect("a holder");
        assert_eq!(holder.as_str(), "aura-renderer");
    }

    #[test]
    fn no_lease_beside_a_master_is_nobody_holding_the_display() {
        let answer = r#"{"subject":"status","phase":"running","health":"ok","mode":"kiosk","locked":false,"display_master":"rivet-launcher","services":[]}"#;
        assert!(read_holder(answer).expect("readable").is_none());
        // A headless machine sends no master, and its version says it knows
        // the field.
        let headless = r#"{"subject":"status","phase":"running","version":"0.1.86","health":"ok","mode":"headless","services":[]}"#;
        assert!(read_holder(headless).expect("readable").is_none());
    }

    /// **A silence that could be an old supervisor's is refused**, not read as
    /// "nobody": that would be a wrong guess that succeeds.
    #[test]
    fn a_status_that_could_not_have_named_a_holder_is_refused() {
        let answer = r#"{"subject":"status","phase":"running","health":"ok","mode":"kiosk","services":[]}"#;
        let why = read_holder(answer).expect_err("ambiguous silence");
        assert!(why.contains("older than the field"), "{why}");
    }

    #[test]
    fn a_holder_that_is_not_a_service_name_is_refused_before_a_path_is_composed() {
        let answer = r#"{"subject":"status","version":"0.1.86","display_leased":"../../secrets"}"#;
        let why = read_holder(answer).expect_err("not a service name");
        assert!(why.contains("not a service name"), "{why}");
    }

    #[test]
    fn a_status_about_another_subject_is_refused() {
        let answer = r#"{"subject":"file","display_leased":"aura-renderer"}"#;
        let why = read_holder(answer).expect_err("another subject");
        assert!(why.contains("status"), "{why}");
    }

    // -- the no-holder outcome, against a fake machine --------------------

    /// A machine that answers `get status` with `status`, and panics on any
    /// other call.
    struct StatusOnly {
        status: &'static str,
        calls: std::cell::RefCell<Vec<String>>,
    }

    impl CallsTools for StatusOnly {
        fn call_tool(&self, tool: &str, arguments: &str) -> Result<String, String> {
            self.calls.borrow_mut().push(format!("{tool} {arguments}"));
            assert!(
                arguments.contains("status") && !arguments.contains("file"),
                "only get status may be sent, and this was: {tool} {arguments}"
            );
            Ok(self.status.to_string())
        }
    }

    /// **Nobody holding the display fetches nothing, writes nothing, and is
    /// not an error.** The `--out` and `--qoi` paths are in a scratch
    /// directory and must still not exist afterwards.
    #[test]
    fn no_holder_fetches_nothing_writes_nothing_and_succeeds() {
        let scratch = crate::scratch::absent("screenshot-no-holder");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("a scratch directory");
        let png = scratch.join("frame.png");
        let qoi = scratch.join("frame.qoi");

        let mut words = trust();
        words.push("--holder");
        let mut owned: Vec<String> = words.iter().map(|w| (*w).to_string()).collect();
        owned.extend([
            "--out".to_string(),
            png.display().to_string(),
            "--qoi".to_string(),
            qoi.display().to_string(),
        ]);
        let args = parse(&owned).expect("valid");

        let machine = StatusOnly {
            status: r#"{"subject":"status","version":"0.1.86","mode":"kiosk","locked":false,"display_master":"rivet-launcher","services":[]}"#,
            calls: std::cell::RefCell::new(Vec::new()),
        };
        let outcome = fetch(&machine, &args).expect("no holder is not an error");
        assert!(matches!(outcome, Outcome::NoHolder), "{outcome:?}");
        assert_eq!(machine.calls.borrow().len(), 1, "{:?}", machine.calls.borrow());
        assert!(!png.exists(), "a PNG was written with nothing fetched");
        assert!(!qoi.exists(), "a QOI was written with nothing fetched");

        let said = report(&args, &outcome);
        assert!(said.contains("NOTHING WAS FETCHED"), "{said}");
        let _ = std::fs::remove_dir_all(&scratch);
    }

    // -- the JSON a program reads ------------------------------------------

    fn fetched(holder: Option<&str>, modified: Option<i64>) -> Outcome {
        Outcome::Fetched(Fetched {
            holder: holder.map(str::to_string),
            board_path: "/state/app/aura-renderer/data/screen.qoi".to_string(),
            frame_modified_ms: modified,
            size_bytes: 67_219,
            width: 1920,
            height: 1080,
            channels: 3,
            opaque: true,
            out: PathBuf::from(r"C:\shots\a.png"),
            png_bytes: 6_221_880,
            qoi: None,
        })
    }

    /// **The shape, read back by a JSON parser** rather than compared as text,
    /// so a key order change is not a failure and a missing key or a wrong
    /// type is.
    #[test]
    fn the_json_for_a_fetched_frame_carries_every_key_with_its_type() {
        let text = outcome_json(&fetched(Some("aura-renderer"), Some(1_788_318_718_168)));
        assert!(!text.contains('\n'), "one line: {text}");
        let json = rivet_core::json::parse(&text).expect("valid JSON");
        assert_eq!(json.get("outcome").and_then(Json::as_str), Some(FETCHED));
        assert_eq!(json.get("holder").and_then(Json::as_str), Some("aura-renderer"));
        assert_eq!(
            json.get("board_path").and_then(Json::as_str),
            Some("/state/app/aura-renderer/data/screen.qoi")
        );
        assert_eq!(json.get("frame_modified_ms"), Some(&Json::Integer(1_788_318_718_168)));
        // A Windows path's backslashes survive the escaping.
        assert_eq!(json.get("out").and_then(Json::as_str), Some(r"C:\shots\a.png"));
    }

    #[test]
    fn the_json_for_no_holder_is_the_same_keys_all_null() {
        let text = outcome_json(&Outcome::NoHolder);
        let json = rivet_core::json::parse(&text).expect("valid JSON");
        assert_eq!(json.get("outcome").and_then(Json::as_str), Some(NO_HOLDER));
        for key in ["holder", "board_path", "frame_modified_ms", "out"] {
            assert_eq!(json.get(key), Some(&Json::Null), "{key} in {text}");
        }
    }

    /// Without `--holder` nobody was asked, so `holder` is null beside a
    /// fetched frame; a frame with no timestamp says null rather than zero.
    #[test]
    fn the_json_without_holder_or_timestamp_says_null_for_both() {
        let text = outcome_json(&fetched(None, None));
        let json = rivet_core::json::parse(&text).expect("valid JSON");
        assert_eq!(json.get("outcome").and_then(Json::as_str), Some(FETCHED));
        assert_eq!(json.get("holder"), Some(&Json::Null), "{text}");
        assert_eq!(json.get("frame_modified_ms"), Some(&Json::Null), "{text}");
    }

    /// **The default path is composed from the two constants that own its
    /// halves**, and this is the check that the composition still produces what
    /// a board actually has. It is written out literally on purpose: if
    /// `rivet_core::service::data_dir` or `rivet_core::compositor::PROVIDER`
    /// changes, the composed path changes with it and this test says so rather
    /// than the verb quietly asking for a file that is not there.
    ///
    /// **It was `rivet-launcher`'s until 2026-09-20** — `ADR-058` §5. The
    /// launcher cannot see the screen any more; it draws into a surface like
    /// every other client, and the compositor is the one program that has the
    /// whole frame. `cmd/rivet-displayd/src/capture.rs` composes the same path
    /// from the same function, which is the agreement this asserts.
    #[test]
    fn the_default_path_is_the_one_the_compositor_writes() {
        assert_eq!(
            default_capture_path(),
            "/state/app/rivet-displayd/data/screen.qoi"
        );
    }

    #[test]
    fn the_three_paths_are_all_overridable() {
        let mut words = trust();
        words.extend(["--path", "/state/app/aura/data/frame.qoi", "--out", "a.png", "--qoi", "b.qoi"]);
        let parsed = parse_of(&words).expect("valid");
        assert_eq!(parsed.frame, Frame::Path("/state/app/aura/data/frame.qoi".to_string()));
        assert_eq!(parsed.out, PathBuf::from("a.png"));
        assert_eq!(parsed.qoi, Some(PathBuf::from("b.qoi")));
    }

    /// `ADR-025` §2's pairing, checked by the same code `run` and `deploy` use.
    #[test]
    fn half_the_trust_model_is_refused_before_anything_connects() {
        let refused = parse_of(&["--host", "pi:7777", "--identity", "./id"])
            .expect_err("no pin");
        assert!(refused.contains("--pin"), "{refused}");
    }

    #[test]
    fn an_option_this_verb_does_not_take_is_refused_rather_than_ignored() {
        let mut words = trust();
        words.extend(["--rvt", "x.rvt"]);
        let refused = parse_of(&words).expect_err("not an option here");
        assert!(refused.contains("--rvt"), "{refused}");
    }

    /// **Every port in the help is the control plane's own**, on the same
    /// argument `run`'s test makes: `PLAN.md` §92's fourth defect was three
    /// ports in three places, two of them wrong, and an operator copying an
    /// example had no way to know which.
    #[test]
    fn every_port_in_the_help_is_the_control_planes_default() {
        let usage = usage();
        let default = rivet_config::control_plane::DEFAULT_PORT.to_string();
        let mut found = 0usize;
        for (at, _) in usage.match_indices(':') {
            let digits: String = usage[at + 1..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            if digits.is_empty() {
                continue;
            }
            found += 1;
            assert_eq!(
                digits, default,
                "this help shows port {digits}, and the control plane's is {default}:\n{usage}"
            );
        }
        assert!(found >= 1, "no port was found in the usage at all:\n{usage}");
    }

    /// **The help says this does not press the button.** It is the one thing a
    /// reader will assume wrongly, and assuming it wrongly costs them a
    /// screenshot of the wrong moment that looks entirely convincing.
    #[test]
    fn the_help_says_it_converts_rather_than_captures() {
        let usage = usage();
        assert!(usage.contains("DOES NOT TAKE THE PICTURE"), "{usage}");
        assert!(usage.contains("apply input capture"), "{usage}");
        assert!(usage.contains("REFUSES"), "{usage}");
        // And that the PNG is large on purpose, so nobody reports it as a bug.
        assert!(usage.contains("stored blocks"), "{usage}");
    }

    // -- reading the machine's answer ------------------------------------
    //
    // `read_answer` is the half of this verb that needs no board, and it is the
    // half that can be wrong in ways a picture would not reveal.

    const PATH: &str = "/state/app/rivet-launcher/data/screen.qoi";

    /// One `get file` answer, in the shape the machine composes, with `content`
    /// carrying whatever bytes are given.
    fn answer_for(bytes: &[u8]) -> String {
        format!(
            r#"{{"subject":"file","path":"{PATH}","present":true,"type":"file","size_bytes":{},"modified_ms":1788318718168,"content":"{}","content_encoding":"base64"}}"#,
            bytes.len(),
            rivet_core::base64::encode(bytes),
        )
    }

    #[test]
    fn the_bytes_come_back_exactly_as_the_machine_read_them() {
        let bytes: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
        let capture = read_answer(&answer_for(&bytes), PATH).expect("a well-formed answer");
        assert_eq!(capture.bytes, bytes);
        assert_eq!(capture.size_bytes, 1000);
        assert_eq!(capture.modified_ms, Some(1_788_318_718_168));
    }

    /// **An answer about another file is refused.** One connection can carry
    /// several calls, and an answer bound to the wrong one would produce a
    /// picture of a different file with nothing saying so.
    #[test]
    fn an_answer_about_another_path_is_refused() {
        let answer = answer_for(b"whatever").replace(PATH, "/state/other");
        let why = read_answer(&answer, PATH).expect_err("a different file");
        assert!(why.contains("different file"), "{why}");
    }

    /// A machine that answers `get file` in fixed-size windows, so the loop
    /// that reassembles them can be exercised without a board.
    struct Windowed {
        bytes: Vec<u8>,
        per_call: usize,
        calls: std::cell::RefCell<Vec<String>>,
    }

    impl CallsTools for Windowed {
        fn call_tool(&self, _tool: &str, arguments: &str) -> Result<String, String> {
            self.calls.borrow_mut().push(arguments.to_string());
            // The offset is the fourth word of the translated call, when there
            // is one. Parsed out of the JSON arguments the same way a machine
            // would receive it.
            let at = arguments
                .rsplit('"')
                .find_map(|word| word.parse::<usize>().ok())
                .unwrap_or(0);
            let take = self.per_call.min(self.bytes.len().saturating_sub(at));
            let piece = &self.bytes[at..at + take];
            Ok(format!(
                r#"{{"subject":"file","path":"{PATH}","present":true,"type":"file",
                   "size_bytes":{size},"modified_ms":1788318718168,"content":"{content}",
                   "content_encoding":"base64","content_offset":{at},"content_bytes":{take},
                   "content_complete":{complete}}}"#,
                size = self.bytes.len(),
                content = rivet_core::base64::encode(piece),
                complete = at + take == self.bytes.len(),
            ))
        }
    }

    /// **A capture larger than one answer is fetched in pieces** — the defect
    /// this loop exists for, 2026-09-21.
    ///
    /// `rivet-01` drawing Contours produced a 1,090,017-byte QOI against a
    /// one-mebibyte cap, and this verb — whose entire job is to fetch that
    /// file — failed. Not for a broken screen; for an interesting one.
    #[test]
    fn a_capture_larger_than_one_answer_is_read_in_pieces_and_reassembled() {
        let bytes: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
        let machine = Windowed {
            bytes: bytes.clone(),
            per_call: 2048,
            calls: std::cell::RefCell::new(Vec::new()),
        };
        let capture = read_windows(&machine, PATH).expect("it must reassemble");
        assert_eq!(capture.bytes, bytes, "the pieces did not reassemble");
        assert_eq!(capture.size_bytes, 5000);
        assert_eq!(
            machine.calls.borrow().len(),
            3,
            "5000 bytes in 2048-byte windows is three reads"
        );
    }

    /// A machine that answers from ONE of two frames, switching to the second
    /// after `switch_after` calls and staying there — so a capture read that
    /// straddles the switch can be exercised without a board or a real
    /// `rivet-displayd` publish. B11.
    struct Swapped {
        /// `(bytes, modified_ms)` for the frame in place before the switch.
        before: (Vec<u8>, i64),
        /// `(bytes, modified_ms)` for the frame in place from `switch_after`
        /// on. `modified_ms` may equal `before`'s — a same-timestamp swap is
        /// exactly the limitation `read_windows_once` documents.
        after: (Vec<u8>, i64),
        per_call: usize,
        switch_after: usize,
        calls: std::cell::RefCell<usize>,
    }

    impl CallsTools for Swapped {
        fn call_tool(&self, _tool: &str, arguments: &str) -> Result<String, String> {
            let mut calls = self.calls.borrow_mut();
            let this_call = *calls;
            *calls += 1;
            let (bytes, modified_ms) =
                if this_call < self.switch_after { &self.before } else { &self.after };
            let at = arguments
                .rsplit('"')
                .find_map(|word| word.parse::<usize>().ok())
                .unwrap_or(0);
            let take = self.per_call.min(bytes.len().saturating_sub(at));
            let piece = &bytes[at..at + take];
            Ok(format!(
                r#"{{"subject":"file","path":"{PATH}","present":true,"type":"file",
                   "size_bytes":{size},"modified_ms":{modified_ms},"content":"{content}",
                   "content_encoding":"base64","content_offset":{at},"content_bytes":{take},
                   "content_complete":{complete}}}"#,
                size = bytes.len(),
                content = rivet_core::base64::encode(piece),
                complete = at + take == bytes.len(),
            ))
        }
    }

    /// **The acceptance case B11 exists for: a same-size overwrite between
    /// two windows of one read.** Before this fix the only cross-window check
    /// was the assembled total against the FINAL window's `size_bytes` —
    /// which a same-size swap satisfies perfectly, so the two frames were
    /// spliced into one "capture" that still decoded, with nothing to say it
    /// was wrong.
    ///
    /// The two frames here are the same length and differ only in
    /// `modified_ms` and content, so a length check alone cannot catch this —
    /// only the per-window metadata comparison can. The swap happens after
    /// the first window, so a two-window read straddles it exactly.
    #[test]
    fn a_same_size_frame_swapped_between_windows_is_never_returned_as_one_capture() {
        let before: Vec<u8> = (0..3000u32).map(|i| (i % 200) as u8).collect();
        let after: Vec<u8> = (0..3000u32).map(|i| ((i * 7) % 200) as u8).collect();
        assert_ne!(before, after, "the two frames must actually differ");
        assert_eq!(before.len(), after.len(), "a SAME-SIZE swap is the case under test");

        let machine = Swapped {
            before: (before.clone(), 1_000_000_000_000),
            after: (after.clone(), 2_000_000_000_000),
            per_call: 1500,
            switch_after: 1,
            calls: std::cell::RefCell::new(0),
        };
        let capture = read_windows(&machine, PATH)
            .expect("the swap happens once; a retry lands entirely on the new frame");
        // The exact check a splice would fail: this must be ONE frame's bytes
        // whole, not the old frame and not some other length built from
        // pieces of both.
        assert_eq!(capture.bytes, after, "must be the whole new frame");
        assert_ne!(capture.bytes, before, "must not be the old frame");
        assert_eq!(capture.bytes.len(), after.len(), "must not be a mix of the two lengths");
    }

    /// **The same case, but the sizes differ too** — the failure mode the
    /// pre-B11 total-length check WOULD have caught on its own, kept as its
    /// own test so the two checks (size, timestamp) are each proven
    /// independently rather than only in combination.
    #[test]
    fn a_changed_size_between_windows_is_never_returned_as_one_capture() {
        let before: Vec<u8> = vec![0xAA; 2000];
        let after: Vec<u8> = vec![0xBB; 3000];

        let machine = Swapped {
            before: (before.clone(), 1_000_000_000_000),
            after: (after.clone(), 1_000_000_000_000), // same modified_ms on purpose
            per_call: 1200,
            switch_after: 1,
            calls: std::cell::RefCell::new(0),
        };
        let capture = read_windows(&machine, PATH)
            .expect("the swap happens once; a retry lands entirely on the new frame");
        assert_eq!(capture.bytes, after, "must be the whole new frame");
        assert_eq!(capture.size_bytes, 3000);
    }

    /// **A read that would need a THIRD frame to settle is refused, not
    /// retried forever.** One retry is the contract; a board answering with a
    /// new frame on every single call is reported rather than looped on.
    #[test]
    fn a_frame_that_keeps_changing_is_refused_after_one_retry() {
        struct AlwaysChanging {
            calls: std::cell::RefCell<usize>,
        }
        impl CallsTools for AlwaysChanging {
            fn call_tool(&self, _tool: &str, _arguments: &str) -> Result<String, String> {
                let mut calls = self.calls.borrow_mut();
                let n = *calls;
                *calls += 1;
                // A fresh, two-window frame on every single call: window 0
                // never agrees with window 1, on either attempt.
                Ok(format!(
                    r#"{{"subject":"file","path":"{PATH}","present":true,"type":"file",
                       "size_bytes":6,"modified_ms":{ms},"content":"YWJj","content_encoding":"base64",
                       "content_offset":0,"content_bytes":3,"content_complete":false}}"#,
                    ms = 1_000_000_000_000i64 + n as i64,
                ))
            }
        }
        let machine = AlwaysChanging { calls: std::cell::RefCell::new(0) };
        let why = read_windows(&machine, PATH).expect_err("must not settle on a spliced read");
        assert!(why.contains("changed while it was being read, twice in a row"), "{why}");
    }

    /// **A windowed peer that never sends `modified_ms` is read exactly as it
    /// was before this check existed — B11's own "keep today's behaviour"
    /// case, distinct from `a_machine_with_no_window_fields_is_read_the_old_way`'s
    /// single-shot ancient peer.** This one DOES support the byte-offset
    /// window (`content_offset`/`content_bytes`/`content_complete`), the way
    /// a current machine does, and simply never has a timestamp for this
    /// file to report — `read_answer` already treats an absent
    /// `modified_ms` as `None`, and the new per-window check only compares
    /// two timestamps when BOTH windows have one.
    struct WindowedNoTimestamp {
        bytes: Vec<u8>,
        per_call: usize,
        calls: std::cell::RefCell<usize>,
    }

    impl CallsTools for WindowedNoTimestamp {
        fn call_tool(&self, _tool: &str, arguments: &str) -> Result<String, String> {
            *self.calls.borrow_mut() += 1;
            let at = arguments
                .rsplit('"')
                .find_map(|word| word.parse::<usize>().ok())
                .unwrap_or(0);
            let take = self.per_call.min(self.bytes.len().saturating_sub(at));
            let piece = &self.bytes[at..at + take];
            Ok(format!(
                r#"{{"subject":"file","path":"{PATH}","present":true,"type":"file",
                   "size_bytes":{size},"content":"{content}","content_encoding":"base64",
                   "content_offset":{at},"content_bytes":{take},"content_complete":{complete}}}"#,
                size = self.bytes.len(),
                content = rivet_core::base64::encode(piece),
                complete = at + take == self.bytes.len(),
            ))
        }
    }

    #[test]
    fn a_windowed_peer_with_no_modified_ms_at_all_is_read_without_a_spurious_retry() {
        let bytes: Vec<u8> = (0..4000u32).map(|i| (i % 200) as u8).collect();
        let machine = WindowedNoTimestamp {
            bytes: bytes.clone(),
            per_call: 1500,
            calls: std::cell::RefCell::new(0),
        };
        let capture = read_windows(&machine, PATH).expect("no timestamp is not a refusal");
        assert_eq!(capture.bytes, bytes);
        assert_eq!(
            *machine.calls.borrow(),
            3,
            "4000 bytes in 1500-byte windows is three reads — a spurious retry would double it"
        );
    }

    /// **A machine that never says it is complete is reported, not asked for
    /// ever.** `content_complete` comes from the other end, so the loop needs
    /// its own bound.
    #[test]
    fn a_machine_that_never_finishes_is_refused_rather_than_looped_on() {
        struct NeverDone;
        impl CallsTools for NeverDone {
            fn call_tool(&self, _tool: &str, _arguments: &str) -> Result<String, String> {
                Ok(format!(
                    r#"{{"subject":"file","path":"{PATH}","present":true,"type":"file",
                       "size_bytes":99999999,"content":"YWJj","content_encoding":"base64",
                       "content_offset":0,"content_bytes":3,"content_complete":false}}"#
                ))
            }
        }
        let why = read_windows(&NeverDone, PATH).expect_err("it must give up");
        assert!(why.contains("did not end within"), "{why}");
    }

    /// **An answer from a machine that predates the window is read whole**, so
    /// a newer workstation still fetches from an older board.
    #[test]
    fn a_machine_with_no_window_fields_is_read_the_old_way() {
        struct Older;
        impl CallsTools for Older {
            fn call_tool(&self, _tool: &str, _arguments: &str) -> Result<String, String> {
                Ok(format!(
                    r#"{{"subject":"file","path":"{PATH}","present":true,"type":"file",
                       "size_bytes":3,"content":"YWJj","content_encoding":"base64"}}"#
                ))
            }
        }
        let capture = read_windows(&Older, PATH).expect("an older machine must still answer");
        assert_eq!(capture.bytes, b"abc");
        assert!(capture.complete);
    }

    /// **A length that disagrees with what the machine counted is refused.** Two
    /// layers of encoding stand between the two numbers and a mistake in either
    /// produces a picture rather than a failure.
    #[test]
    fn a_length_the_machine_disagrees_with_is_refused() {
        let answer = answer_for(b"12345678").replace("\"size_bytes\":8", "\"size_bytes\":9");
        let why = read_answer(&answer, PATH).expect_err("the lengths disagree");
        assert!(why.contains("9 bytes"), "{why}");
        assert!(why.contains("8 arrived"), "{why}");
    }

    #[test]
    fn nothing_at_the_path_says_how_to_take_a_capture() {
        let answer = format!(
            r#"{{"subject":"file","path":"{PATH}","present":false}}"#
        );
        let why = read_answer(&answer, PATH).expect_err("nothing there");
        assert!(why.contains("nothing at"), "{why}");
        assert!(why.contains("apply input capture"), "{why}");
    }

    /// **The machine's own reason is carried through verbatim.** It is more
    /// specific than anything this end could compose about a file it has never
    /// seen — a file over the 1 MiB read limit says exactly that, with the
    /// number.
    #[test]
    fn an_omitted_content_carries_the_machines_own_sentence() {
        let answer = format!(
            r#"{{"subject":"file","path":"{PATH}","present":true,"type":"file","size_bytes":9999999,"content_omitted":"this file holds 9999999 bytes and this operation reads a file whole"}}"#
        );
        let why = read_answer(&answer, PATH).expect_err("no content");
        assert!(why.contains("reads a file whole"), "{why}");
    }

    /// A path holding text is not a capture, and saying which is more useful
    /// than saying the QOI magic is wrong two steps later.
    #[test]
    fn a_text_file_is_refused_before_it_reaches_the_qoi_decoder() {
        let answer = format!(
            r#"{{"subject":"file","path":"{PATH}","present":true,"type":"file","size_bytes":3,"content":"abc","content_encoding":"text"}}"#
        );
        let why = read_answer(&answer, PATH).expect_err("text is not a capture");
        assert!(why.contains("never reads as text"), "{why}");
    }

    #[test]
    fn a_directory_is_refused_by_its_type() {
        let answer = format!(
            r#"{{"subject":"file","path":"{PATH}","present":true,"type":"directory","entry_count":3}}"#
        );
        let why = read_answer(&answer, PATH).expect_err("a directory");
        assert!(why.contains("is a directory"), "{why}");
    }

    #[test]
    fn an_answer_that_is_not_json_is_refused_with_what_arrived() {
        let why = read_answer("not json at all", PATH).expect_err("not JSON");
        assert!(why.contains("not JSON"), "{why}");
        assert!(why.contains("not json at all"), "it quotes what arrived: {why}");
    }

    /// Content the machine calls base64 and which is not decodes to nothing
    /// rather than to plausible bytes.
    #[test]
    fn content_that_is_not_base64_is_refused_rather_than_read_leniently() {
        let answer = format!(
            r#"{{"subject":"file","path":"{PATH}","present":true,"type":"file","size_bytes":3,"content":"a*b=","content_encoding":"base64"}}"#
        );
        let why = read_answer(&answer, PATH).expect_err("not base64");
        assert!(why.contains("does not decode"), "{why}");
    }

    // -- the report ------------------------------------------------------

    #[test]
    fn the_report_says_the_frame_was_not_taken_by_this_command() {
        let args = parse_of(&trust()).expect("valid");
        let said = report(&args, &fetched(None, Some(1_788_318_718_168)));
        assert!(said.contains("Nothing was pressed"), "{said}");
        assert!(said.contains("BY THE BOARD'S CLOCK"), "{said}");
        assert!(said.contains("1920×1080"), "{said}");
        assert!(said.contains("a.png"), "{said}");
        assert!(!said.contains("holder "), "no holder was asked for: {said}");

        let said = report(&args, &fetched(Some("aura-renderer"), None));
        assert!(said.contains("aura-renderer holds the display"), "{said}");
    }
}
