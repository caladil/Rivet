//! **How a program reaches the compositor**, and how it is refused.
//!
//! `ADR-036` §5: an application is *handed* what it needs. That binds this
//! program in both directions — it is handed its listening socket rather than
//! binding one, and a client is handed a connection rather than discovering it.
//!
//! # Nothing here reads `WAYLAND_DISPLAY` or `XDG_RUNTIME_DIR`
//!
//! Wayland's own discovery has a client **search** a runtime directory for a
//! compositor, which is the opposite of how every other socket on this machine
//! works. `rivet_core::compositor` states the decision; this module is it in
//! code. The supervisor binds `/run/rivet/listen/rivet-displayd/wayland`, hands
//! this process the descriptor as `LISTEN_FDS`, and chowns the file to the one
//! application that declared `connects = ["rivet-displayd:wayland"]`. A program
//! that did not declare it cannot open the path at all — the refusal is the
//! file mode, which is `ADR-015` §5's `0600` and not a check written here.

use std::os::unix::net::UnixListener;
use std::sync::Arc;

use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction};
use smithay::reexports::wayland_server::Display;

use crate::state::{ClientState, Rivet};
use crate::Loop;

/// Put the compositor's two descriptors into the event loop: the socket
/// clients arrive on, and the display's own.
///
/// # Errors
///
/// No socket was handed over and none could be bound, or a source the loop
/// refused.
pub fn wire(handle: &LoopHandle<'static, Loop>, display: &mut Display<Rivet>) -> Result<(), String> {
    let listener = listener()?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("the compositor socket could not be made non-blocking: {e}"))?;

    handle
        .insert_source(
            Generic::new(listener, Interest::READ, Mode::Level),
            |_readiness, listener, data| {
                // **Every accept is drained.** Level-triggered readiness with
                // a single accept per wake-up is a livelock waiting for two
                // clients to connect in the same millisecond.
                loop {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            match data
                                .state
                                .handle
                                .insert_client(stream, Arc::new(ClientState::default()))
                            {
                                Ok(_) => println!("rivet-displayd: a client connected"),
                                Err(why) => {
                                    println!("rivet-displayd: a client was refused — {why}");
                                }
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(e) => {
                            println!("rivet-displayd: accept failed — {e}");
                            break;
                        }
                    }
                }
                Ok(PostAction::Continue)
            },
        )
        .map_err(|e| format!("the compositor socket could not be watched: {e}"))?;

    // **The capture socket** — `ADR-058` §5. A separate endpoint from the
    // Wayland one and deliberately trivial: a connection asks, a line comes
    // back. It is here rather than as a Wayland request because the caller is
    // an OPERATOR over the control plane, and an answer that routed through a
    // client would be unavailable exactly when a client is wedged.
    if let Some(listener) = capture_listener() {
        if let Err(e) = listener.set_nonblocking(true) {
            println!("rivet-displayd: the capture socket could not be made non-blocking — {e}");
        }
        handle
            .insert_source(
                Generic::new(listener, Interest::READ, Mode::Level),
                |_readiness, listener, data| {
                    loop {
                        match listener.accept() {
                            Ok((stream, _)) => answer_capture(stream, data),
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                            Err(e) => {
                                println!("rivet-displayd: capture accept failed — {e}");
                                break;
                            }
                        }
                    }
                    Ok(PostAction::Continue)
                },
            )
            .map_err(|e| format!("the capture socket could not be watched: {e}"))?;
    }

    let poll = display
        .backend()
        .poll_fd()
        .try_clone_to_owned()
        .map_err(|e| format!("the display descriptor could not be duplicated: {e}"))?;

    handle
        .insert_source(
            Generic::new(poll, Interest::READ, Mode::Level),
            |_readiness, _fd, _data| {
                // The dispatch itself happens in `main`'s loop, which owns the
                // `Display`. This source exists so the loop WAKES when a client
                // writes — without it the compositor would only notice a
                // request when the 16 ms poll expired, which is a whole frame
                // of latency on every click.
                Ok(PostAction::Continue)
            },
        )
        .map_err(|e| format!("the display descriptor could not be watched: {e}"))?;

    Ok(())
}

/// The capture socket, if the supervisor handed one over.
///
/// **No fallback to binding it**, unlike the Wayland one: a probe run without a
/// supervisor has no operator asking for screenshots, and binding a second path
/// would put a file on `/run` that nothing would ever connect to.
fn capture_listener() -> Option<UnixListener> {
    let socket = rivet_sdk::listen::socket(rivet_core::compositor::CAPTURE_ENDPOINT)?;
    let listener = socket.into_unix_listener()?;
    println!(
        "rivet-displayd: captures on request, into {}",
        crate::capture::path()
    );
    Some(listener)
}

/// Start a picture: draw it and queue its copy, then let the frame loop carry
/// on. `main`'s loop collects it once [`crate::capture::Pending::ready`], and
/// [`complete_capture`] answers the caller from a thread of its own.
///
/// **The answer is a sentence and the failure is too.** This socket is the
/// operator's, reached from `rivet apply` over the control plane, and a
/// refusal that arrived as a closed connection would be read as *the machine
/// is not answering* rather than as *the capture did not work, and here is
/// why*.
fn answer_capture(stream: std::os::unix::net::UnixStream, data: &mut Loop) {
    use std::sync::atomic::Ordering;

    // One at a time: a second press arriving while the first is still being
    // taken is told so, rather than racing it to the same file.
    if data.capture.is_some() || data.capture_busy.load(Ordering::SeqCst) {
        answer_off_thread(stream, "failed a capture is already being taken; ask again\n".to_string());
        return;
    }

    // **No display, no picture, and the caller is told so at once** —
    // `.rulings/` R-236 makes a kiosk with nothing plugged in an ordinary
    // state. Drawing the scene anyway would photograph a size nobody chose,
    // and leaving the last file in place would answer with a stale frame:
    // `cargo rivet screenshot` fetches whatever the path holds, so the last
    // picture is withdrawn and that tool reports that there is none.
    let Some(output) = data.kms.head.as_ref().map(|head| head.output.clone()) else {
        println!("rivet-displayd: a capture was asked for — {}", crate::screen::NO_DISPLAY);
        crate::capture::withdraw();
        answer_off_thread(stream, format!("failed {}\n", crate::screen::NO_DISPLAY));
        return;
    };

    // The linear buffer first — `capture`'s module note — and the readback
    // only for a driver that will not draw into one.
    let drawn = data.kms.capture_buffer().and_then(|target| {
        crate::capture::draw(&mut data.kms.renderer, &output, &mut data.state, target)
    });
    match drawn {
        Ok(drawn) => complete_drawn(drawn, stream, &data.capture_busy),
        Err(why) => {
            println!("rivet-displayd: the capture reads back on the compositor's thread instead — {why}");
            match crate::capture::begin(&mut data.kms.renderer, &output, &mut data.state) {
                Ok(pending) => data.capture = Some((pending, stream)),
                Err(why) => {
                    println!("rivet-displayd: the capture failed — {why}");
                    answer_off_thread(stream, format!("failed {why}\n"));
                }
            }
        }
    }

    // **The frame loop draws again next vblank.** Both paths bind a buffer of
    // their own and unbind it, so the compositor's own target has to be
    // re-established before the next frame.
    data.state.redraw = true;
}

/// Wait for, read, encode, publish and answer a capture drawn into a linear
/// buffer, on a thread of its own; `busy` is set until it is done.
fn complete_drawn(
    drawn: crate::capture::Drawn,
    stream: std::os::unix::net::UnixStream,
    busy: &Arc<std::sync::atomic::AtomicBool>,
) {
    use std::sync::atomic::Ordering;

    busy.store(true, Ordering::SeqCst);
    let done = Arc::clone(busy);
    let finish = move || {
        let answer = match crate::capture::finish_drawn(drawn) {
            Ok(finished) => {
                println!("rivet-displayd: captured {}", finished.journal);
                format!("ok {}\n", finished.answer)
            }
            Err(why) => {
                println!("rivet-displayd: the capture failed — {why}");
                format!("failed {why}\n")
            }
        };
        reply(stream, &answer);
        done.store(false, Ordering::SeqCst);
    };
    if let Err(e) = std::thread::Builder::new().name("capture".into()).spawn(finish) {
        busy.store(false, Ordering::SeqCst);
        println!("rivet-displayd: the capture could not be finished — no thread for it: {e}");
    }
}

/// Encode, publish and answer a collected capture, on a thread of its own.
pub fn complete_capture(frame: crate::capture::Frame, stream: std::os::unix::net::UnixStream) {
    let finish = move || {
        let answer = match crate::capture::finish(&frame) {
            Ok(done) => {
                println!("rivet-displayd: captured {}", done.journal);
                format!("ok {}\n", done.answer)
            }
            Err(why) => {
                println!("rivet-displayd: the capture failed — {why}");
                format!("failed {why}\n")
            }
        };
        reply(stream, &answer);
    };
    if let Err(e) = std::thread::Builder::new().name("capture".into()).spawn(finish) {
        // The closure moved into the failed spawn and is gone with it, so the
        // caller cannot be answered; the journal can still say why.
        println!("rivet-displayd: the capture could not be finished — no thread for it: {e}");
    }
}

/// Answer a capture caller with `answer`, on a thread of its own, because
/// [`reply`] can wait on the caller and the compositor's thread must not.
pub fn answer_off_thread(stream: std::os::unix::net::UnixStream, answer: String) {
    if let Err(e) = std::thread::Builder::new()
        .name("capture-answer".into())
        .spawn(move || reply(stream, &answer))
    {
        println!("rivet-displayd: a capture caller could not be answered — no thread for it: {e}");
    }
}

/// Read the caller's request, then write `answer`.
///
/// **The request is READ before the answer is written**, and skipping it is
/// not harmless. A socket closed with unread bytes still in its receive queue
/// makes the kernel send **RST instead of FIN**, so the caller's read fails
/// with `ECONNRESET` and never sees the answer that was already written — the
/// picture on disk and the caller told the machine had hung up. The read waits
/// up to 250 ms for a caller that has not sent yet, which is why this runs on
/// a thread of its own and never on the compositor's.
///
/// **The content is not checked**, deliberately: any connection on this socket
/// means *take one*. `rivet_core::compositor::CAPTURE_REQUEST` exists so a
/// person poking this with `nc` has something to type, not so this can refuse
/// the wrong word.
fn reply(mut stream: std::os::unix::net::UnixStream, answer: &str) {
    use std::io::{Read, Write};

    let mut asked = [0u8; 64];
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(250)));
    let _ = stream.read(&mut asked);
    let _ = stream.write_all(answer.as_bytes());
}

/// The listening socket, from the supervisor if it handed one over.
///
/// # The fallback is for a board with no supervisor, and it says so
///
/// `cargo rivet run` and the probes in `examples/probes/` start a program
/// directly, with no `LISTEN_FDS` and no bound socket. Refusing there would
/// make the compositor untestable except through a full install, which is the
/// loop `ADR-026` §7 spent a week shortening. So the path is bound here
/// instead — **and the journal says which of the two happened**, because
/// "handed" and "bound it myself" are different machines and a line that did
/// not distinguish them would make the first hour of every investigation a
/// guess.
fn listener() -> Result<UnixListener, String> {
    if let Some(socket) = rivet_sdk::listen::socket(rivet_core::compositor::ENDPOINT) {
        if let Some(listener) = socket.into_unix_listener() {
            println!("rivet-displayd: the supervisor handed over the compositor socket");
            return Ok(listener);
        }
        return Err(format!(
            "the supervisor handed over something named `{}` that is not a unix socket",
            rivet_core::compositor::ENDPOINT
        ));
    }

    let path = rivet_core::compositor::socket_path();
    if let Some(parent) = std::path::Path::new(&path).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("{}: the socket directory could not be created: {e}", parent.display()))?;
    }
    // A socket file outlives the process that bound it, and `/run` is a tmpfs
    // only across boots — a restart within one boot finds its own corpse and
    // `bind` answers `EADDRINUSE`.
    let _ = std::fs::remove_file(&path);

    let listener = UnixListener::bind(&path)
        .map_err(|e| format!("{path}: the compositor socket could not be bound: {e}"))?;
    println!("rivet-displayd: NOT handed a socket — bound {path} directly, which means no supervisor started this");
    Ok(listener)
}
