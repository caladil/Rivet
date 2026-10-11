//! **Where a program goes to be given a surface** — `ADR-058` §4 and §5.
//!
//! Two strings and a path derived from them, here rather than in
//! `cmd/rivet-displayd` for `ADR-011`'s reason: both ends of this socket have
//! to agree. The supervisor binds it and chowns it to whoever declared
//! `connects`; the compositor accepts on the descriptor it is handed; a client
//! opens the path. Two spellings would fail as *nothing happened*, which is the
//! worst way for a screen to fail.
//!
//! # There is no protocol here, and that is the decision
//!
//! [`crate::kiosk`] carries a request type and an answer type because Rivet
//! invented that conversation. **This module carries no types at all**, because
//! the conversation on this socket is **Wayland** — ruled 2026-09-20,
//! `ADR-058` §5. A `rivet_core::surface` existed for one afternoon and was
//! removed the same day: a second surface protocol beside Wayland's would be a
//! second answer to a settled question, and the tree would have had to keep
//! both correct for ever.
//!
//! # Why not `WAYLAND_DISPLAY`
//!
//! Wayland's own discovery is `XDG_RUNTIME_DIR` plus a socket name, which means
//! a client **searches** for its compositor. Rivet does not work that way —
//! `ADR-036` §5's first article is that an application is *handed* what it
//! needs, and `ADR-042` §7 already refuses every application the right to bind a
//! port for the same reason.
//!
//! So this is an ordinary `provides = ["socket:wayland"]` endpoint and nothing
//! about it is special-cased: [`socket_path`] is
//! [`crate::service::provided_socket_path`], the supervisor binds and chowns it
//! by the path it already uses for every other provider, and a client that did
//! not declare `connects` cannot open it. **Nothing on this machine reads
//! `XDG_RUNTIME_DIR` or `WAYLAND_DISPLAY`.**

/// The service that provides the compositor's socket — the `<provider>` half of
/// a manifest's `connects = ["rivet-displayd:wayland"]`.
///
/// **This is also the supervisor's service name for it**, exactly as
/// [`crate::input::PROVIDER`] is the launcher's: one string, so a definition and
/// a manifest cannot disagree about which program is meant.
pub const PROVIDER: &str = "rivet-displayd";

/// **The endpoint name, not a path** — [`crate::input::ENDPOINT`]'s rule.
pub const ENDPOINT: &str = "wayland";

/// **The launcher's Wayland `app_id`**, which is how the compositor tells the
/// kiosk's shell from the applications it hands the screen to.
///
/// The launcher is kept beneath every other window, whichever was created
/// first. The two can start together: at boot the supervisor resumes the
/// application a kiosk was showing before, while the launcher starts and
/// selects `[kiosk] auto-run`, and a stack that ordered windows by creation
/// alone put whichever came second on top. One spelling here, used by the
/// launcher that sets it and the compositor that reads it.
pub const LAUNCHER_APP_ID: &str = "org.rivet.launcher";

/// **Take a picture of the whole screen** — `ADR-058` §5.
///
/// A second endpoint on the same provider, and a deliberately tiny one: a
/// connection asks, a line comes back saying where the file is. Nothing is
/// carried over it.
///
/// # Why this is not a Wayland request
///
/// Wayland has `wlr-screencopy`, and that is the right answer for a CLIENT
/// that wants pixels — a screen recorder, a remote-desktop server. This is a
/// different question: an **operator** over the control plane asking what is on
/// the screen, where the answer must not depend on any client being alive,
/// willing, or correct. A capture that a wedged application could refuse is
/// exactly the capture you cannot get when you need it most.
///
/// # Why it is not an application's job
///
/// `ADR-036` §5's first article, read the other way round. Before `ADR-058` a
/// capture came from whoever held DRM master, which was the launcher — so it
/// only ever photographed the launcher's own frame and never a lessee's. The
/// compositor is the first program on this machine that can see **the screen**
/// rather than one program's idea of it, and asking each application to
/// volunteer a picture would be `ADR-011`'s N implementations of one thing,
/// each able to lie.
pub const CAPTURE_ENDPOINT: &str = "capture";

/// What a caller writes on [`CAPTURE_ENDPOINT`] to ask for one.
///
/// A word and a newline, like [`crate::kiosk`]'s requests: this is a socket a
/// person may reasonably poke with `nc` while working out why a screen is
/// wrong, and a binary framing would make that impossible for no gain.
pub const CAPTURE_REQUEST: &str = "capture\n";

/// The file a capture is written to, inside the compositor's own data
/// directory — `/state/app/rivet-displayd/data/screen.qoi`.
///
/// **The same name `rivet_sdk::screen::Screen::capture` writes**, so
/// `cargo rivet screenshot` finds it by the convention it already has rather
/// than by a special case for this program.
pub const CAPTURE_FILE: &str = "screen.qoi";

/// Where the socket is, for the end that opens it by path.
///
/// The provider never calls this: it is handed the descriptor. A client has
/// nothing to be handed, so it computes the path the supervisor bound — which
/// is [`crate::service::provided_socket_path`] and not a constant, because a
/// constant would be a second copy of a layout the supervisor owns.
#[must_use]
pub fn socket_path() -> String {
    match crate::service::ServiceName::new(PROVIDER) {
        Some(name) => crate::service::provided_socket_path(&name, ENDPOINT),
        // **Unreachable, and spelled out rather than unwrapped.** [`PROVIDER`]
        // is a constant that satisfies `ServiceName::RULE` and
        // `the_provider_is_a_name_the_supervisor_would_accept` proves it. A
        // `.unwrap()` here would be a panic in a library function the program
        // that may not fail calls — `ADR-017` §3.
        None => format!("{}/{PROVIDER}/{ENDPOINT}", crate::service::LISTEN_DIR),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The provider name is a service name**, and `cmd/rivet-sysd` looks the
    /// program up by it. A rename here without a rename there produces a
    /// machine whose screen is never driven and whose journal says nothing,
    /// because an endpoint nobody provides is not an error until somebody
    /// connects.
    #[test]
    fn the_provider_is_the_service_name_the_supervisor_starts() {
        assert_eq!(PROVIDER, "rivet-displayd");
    }

    /// A manifest writes `connects = ["rivet-displayd:wayland"]`, and that
    /// string is these two constants joined. Spelling it by hand here is the
    /// point: it is what a person types.
    #[test]
    fn the_two_halves_spell_what_a_manifest_declares() {
        assert_eq!(format!("{PROVIDER}:{ENDPOINT}"), "rivet-displayd:wayland");
    }

    /// **The tripwire against inventing a second layout.** An earlier draft of
    /// this module carried `SOCKET_PATH = "/run/rivet/wayland"`, which is a
    /// path the supervisor would never have bound — the socket would have been
    /// created where `provided_socket_path` says and every client would have
    /// found nothing there.
    #[test]
    fn the_path_is_the_one_the_supervisor_binds_for_any_provider() {
        assert_eq!(socket_path(), "/run/rivet/listen/rivet-displayd/wayland");
    }

    /// **Two endpoints, and they must not be the same string.** Both are
    /// bound by the supervisor under one provider, and a collision would make
    /// one silently shadow the other.
    #[test]
    fn the_compositor_offers_two_endpoints_with_different_names() {
        assert_ne!(ENDPOINT, CAPTURE_ENDPOINT);
        assert!(CAPTURE_REQUEST.ends_with('\n'), "a reader waits for a line");
    }

    /// [`socket_path`]'s fallback arm is unreachable only while this holds.
    #[test]
    fn the_provider_is_a_name_the_supervisor_would_accept() {
        assert!(
            crate::service::ServiceName::new(PROVIDER).is_some(),
            "{}",
            crate::service::ServiceName::RULE
        );
    }
}
