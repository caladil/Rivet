//! Listening sockets the supervisor opened — `ADR-036` §3, row 1.
//!
//! `ADR-015` §5.2: *"Rather than granting `CAP_NET_BIND_SERVICE`, Rivet opens
//! the listening socket and passes the file descriptor to the application. The
//! application never holds the privilege and need not know the port."*
//!
//! An application runs as its own unprivileged uid, so **a port below 1024 is
//! unreachable to it** — `bind(2)` answers `EACCES`. Being handed the descriptor
//! is not a convenience; it is the only way a Rivet application serves port 80.
//!
//! # The names are not the manifest's spelling
//!
//! `LISTEN_FDNAMES` is **colon-separated**, and `ADR-015` §3.2 writes sockets as
//! `tcp:80` and `unix:api`. Those two spellings collide: a manifest's `tcp:80`
//! would arrive as *two* names. So the supervisor sends `tcp-<port>` and the
//! bare declared name instead, and keeps the manifest's spelling for anything a
//! person reads.
//!
//! Found by running it — `socket-demo` on a board reported
//! `LISTEN_FDNAMES tcp, 80, unix, api` before the encoding was fixed
//! (`cmd/rivet-sysd/src/kernel.rs`).

/// The first descriptor the supervisor hands over.
///
/// systemd's convention, adopted whole by `ADR-017` §3.4 on the grounds that
/// existing software then works unmodified.
pub const LISTEN_FDS_START: i32 = 3;

/// How many descriptors were passed.
pub const LISTEN_FDS_ENV: &str = "LISTEN_FDS";

/// Which process they were passed to.
///
/// **Checked, not assumed.** The variable is inherited across `exec`, so a child
/// this application spawns would otherwise believe it owns descriptors that
/// belong to its parent — and act on them.
pub const LISTEN_PID_ENV: &str = "LISTEN_PID";

/// One name per descriptor, colon-separated, in the manifest's order.
pub const LISTEN_FDNAMES_ENV: &str = "LISTEN_FDNAMES";

/// A listening socket, already bound and listening.
///
/// # The descriptor is not public, and `Clone` is gone
///
/// **Both were, and the `SAFETY` note on [`Socket::into_tcp_listener`] was false
/// three ways because of it** — the audit of 2026-09-03 (`PLAN.md` §123, Wave
/// 5.3). That note said `sockets()` *"is the only constructor and it never
/// yields the same fd twice"*. It was not: the field was `pub`, so any safe
/// code could write `Socket { fd: 4, name: String::new() }`; the type was
/// `Clone`, so one socket could become two; and `sockets()` is deliberately
/// re-callable and answers the same numbers every time.
///
/// Each of those ends in `from_raw_fd` being called twice on one descriptor,
/// which is two owners and a double close — and on a kiosk application fd 4 may
/// be the display channel, so the second drop closes the screen.
///
/// # A fourth way, closed separately: `from_parts` describes, it does not own
///
/// [`from_parts`] is public and safe and builds a `Socket` for whatever count
/// and names it is handed, with no way to know whether a descriptor at that
/// number was ever opened, let alone opened by this process's supervisor.
/// Consuming `self` and claiming the descriptor once (below) stops two
/// *[`from_parts`]-built* sockets from both adopting the same number, but it
/// cannot stop one of them from adopting a descriptor some unrelated object in
/// this process already owns — [`from_parts`] has no way to see that object.
///
/// So a `Socket` remembers whether it came from the real environment, and only
/// that one is adoptable. See [`from_parts`]'s own doc for what this means for
/// a caller.
#[derive(Debug, PartialEq, Eq)]
pub struct Socket {
    /// The raw descriptor. **Owned by this process**; nothing here closes it
    /// until it is converted, and [`Socket::into_tcp_listener`] converts it at
    /// most once.
    fd: i32,
    /// What the manifest called it — `tcp-8080`, or the declared name of a unix
    /// socket. Empty when the supervisor sent no name for this position.
    pub name: String,
    /// Whether this `Socket` may be turned into an owning type.
    ///
    /// **Only ever `true` for a `Socket` built from this process's real
    /// environment** — see the struct's own note on [`from_parts`]. Checked
    /// before `claim` in every `into_*` conversion, so a `Socket` this crate
    /// cannot vouch for never touches the claim bitmask at all.
    adoptable: bool,
}

/// Which descriptors have already been turned into an owning type.
///
/// **A bitmask rather than a flag on `sockets()`**, and the difference matters.
/// The audit proposed making `sockets()` itself take-once. That would have
/// broken `examples/aura/aura-renderer`, which calls `listen::socket(name)`
/// once per slot in a loop — four calls, of which the last three would have
/// silently answered `None` and left the renderer with one addon socket out of
/// four, with nothing saying why.
///
/// **The unsoundness is not "the list was read twice", it is "one descriptor
/// got two owners"**, so the guard belongs at the conversion. Reading the list
/// as many times as you like is harmless and is what an application actually
/// does.
///
/// A `u64` covers descriptors 0..64. The supervisor passes them from 3 upward
/// and no manifest declares anything near sixty sockets; a descriptor past that
/// is treated as unclaimable rather than claimed, which refuses rather than
/// double-owning.
///
/// **`cfg(any(unix, test))`, following the two callers.** Nothing on a
/// development host can turn a handed descriptor into a listener, so a
/// non-Unix build has nothing to claim — but the bitmask itself is pure
/// arithmetic with a refusal in it, so it is kept in the test binary
/// everywhere. Same reason `flip` and `outputs` carry that gate.
#[cfg(any(unix, test))]
static CLAIMED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Take exclusive ownership of `fd`, once.
#[cfg(any(unix, test))]
fn claim(fd: i32) -> bool {
    use std::sync::atomic::Ordering;
    let Ok(bit) = u32::try_from(fd) else {
        return false;
    };
    if bit >= 64 {
        return false;
    }
    let mask = 1u64 << bit;
    (CLAIMED.fetch_or(mask, Ordering::AcqRel) & mask) == 0
}

impl Socket {
    /// The raw descriptor, to look at.
    ///
    /// **Reading a number is not owning it.** Use it to `poll`, to set an
    /// option, to print. To own it, convert it — and a conversion can happen
    /// only once.
    #[must_use]
    pub const fn fd(&self) -> i32 {
        self.fd
    }

    /// Whether this is a TCP socket, by the supervisor's naming convention.
    ///
    /// **A convention, not a syscall.** The supervisor names TCP sockets
    /// `tcp-<port>` and unix sockets by their declared name, so this is as good
    /// as that convention and no better. An application knows what it declared;
    /// this is for the case where it wants to loop over what it was given.
    #[must_use]
    pub fn is_tcp(&self) -> bool {
        self.name.starts_with("tcp-")
    }

    /// Take ownership of this descriptor as a standard TCP listener.
    ///
    /// **Consuming**, so the descriptor cannot be owned twice — the failure
    /// that produces a listener closed out from under a running server.
    ///
    /// # Why this is here and not in your application
    ///
    /// `FromRawFd` is `unsafe`, and the safety argument is **the platform's,
    /// not the application's**: the supervisor opened this socket, called
    /// `listen` on it, and passed it as descriptor `3 + n` (`ADR-015` §5.2).
    /// An application cannot verify any of that, so an application writing this
    /// block would be asserting something it has no way to know. Written once,
    /// here, where the promise is documented.
    ///
    /// # `None` means somebody already took it, or this one was never provable
    ///
    /// **This returned a `TcpListener` and could not fail, until 2026-09-03.**
    /// The audit found that the argument for that — one constructor, never the
    /// same fd twice — was not true of the code (see [`Socket`]). Consuming
    /// `self` is not enough on its own, because a second `Socket` naming the
    /// same descriptor could always be had.
    ///
    /// So the descriptor is claimed here, once, and a second attempt answers
    /// `None`. **That is a refusal and not a failure**: the descriptor is fine,
    /// somebody else in this program owns it, and handing out a second owner
    /// would end in a double close of a live listener — or, on a kiosk
    /// application where the numbers land that way, of the display channel.
    ///
    /// **A second reason joined it later, and it is checked first.** A
    /// `Socket` built by the public [`from_parts`] rather than read from this
    /// process's real environment describes a descriptor without any proof it
    /// was ever opened — see [`Socket`]'s own note — so it is never adoptable
    /// and this returns `None` for it before `claim` is even consulted. Only a
    /// `Socket` [`sockets`] or [`socket`] handed back may be turned into an
    /// owning type.
    ///
    /// # THE LISTENER IS BLOCKING, AND AN ASYNC CALLER MUST SAY OTHERWISE
    ///
    /// The supervisor opens the socket in its default mode and this hands the
    /// descriptor over unchanged, which is right for a caller that accepts on an
    /// OS thread. **`tokio::net::TcpListener::from_std` does not accept a
    /// blocking listener**, and it does not refuse one either — its own
    /// documentation puts the duty on the caller: *"the caller is responsible
    /// for ensuring that the listener is in non-blocking mode. Otherwise all I/O
    /// operations on the listener will block the thread."* So:
    ///
    /// ```no_run
    /// # fn main() -> std::io::Result<()> {
    /// # let socket = rivet_sdk::listen::socket("tcp-8443").unwrap();
    /// let handed = socket.into_tcp_listener().expect("nothing else took it");
    /// handed.set_nonblocking(true)?;          // <-- this line
    /// // let listener = tokio::net::TcpListener::from_std(handed)?;
    /// # Ok(()) }
    /// ```
    ///
    /// **Both SDK adopters as of 2026-09-05 are asynchronous and one of them is
    /// missing that line.** `Reliquary`'s `cmd/reliquary-rivet` writes
    /// `TcpListener::from_std(handed::listener(LISTEN_NAME)?)?` and never sets
    /// the mode; it has served on a board, because the symptom is a worker
    /// parked inside `accept` between the readiness notification and the
    /// connection rather than an error — which is invisible at one connection at
    /// a time and is a hang under load. Trends' port sets it, in its adapter,
    /// with a comment explaining why. **This paragraph is here because the
    /// second port should not have had to work it out** (`ADR-040` §10.3).
    ///
    /// This method deliberately does **not** set it: a blocking listener is what
    /// an OS-thread accept loop wants, and `rivet-fleetd` and `rivet-mcpd` are
    /// both that shape. Whether the SDK should offer a second, named constructor
    /// instead of a paragraph is `ADR-040` §10.6's open question.
    #[cfg(unix)]
    #[must_use]
    pub fn into_tcp_listener(self) -> Option<std::net::TcpListener> {
        use std::os::fd::FromRawFd as _;
        if !self.adoptable {
            return None;
        }
        if !claim(self.fd) {
            return None;
        }
        // SAFETY: `self.fd` is a descriptor the supervisor opened, listened on
        // and passed to this process (`ADR-015` §5.2) — `self.adoptable` is
        // `true` only for a `Socket` this crate built from that real
        // environment. `claim` above returned true, which it does exactly once
        // per descriptor for the life of this process — so this is the only
        // owner that will ever be made from it.
        #[allow(unsafe_code)]
        Some(unsafe { std::net::TcpListener::from_raw_fd(self.fd) })
    }

    /// The same, for a unix socket.
    ///
    /// See [`Socket::into_tcp_listener`] for why the `unsafe` lives here and
    /// what `None` means.
    #[cfg(unix)]
    #[must_use]
    pub fn into_unix_listener(self) -> Option<std::os::unix::net::UnixListener> {
        use std::os::fd::FromRawFd as _;
        if !self.adoptable {
            return None;
        }
        if !claim(self.fd) {
            return None;
        }
        // SAFETY: as `into_tcp_listener` — an owned, listening descriptor from
        // the supervisor, `self.adoptable` vouches for where it came from, and
        // `claim` has just made this its only owner.
        #[allow(unsafe_code)]
        Some(unsafe { std::os::unix::net::UnixListener::from_raw_fd(self.fd) })
    }

    /// The same, for a **UDP** socket — added 2026-09-06 with
    /// `Socket::Udp` (`PLAN.md` §150.1).
    ///
    /// See [`Socket::into_tcp_listener`] for why the `unsafe` lives here and
    /// what `None` means.
    ///
    /// # Why this method has to exist, and is not a convenience
    ///
    /// **Without it the supervisor can hand out a UDP socket that no
    /// application can safely take.** The descriptor arrives correctly, named
    /// `udp-<port>` in `LISTEN_FDNAMES`, and an author with no method here
    /// reaches for `unsafe { UdpSocket::from_raw_fd(…) }` in their own
    /// crate — which skips `claim`, and skipping it is how one descriptor
    /// acquires two owners and is closed twice. **That is the exact failure
    /// this module's opening paragraph exists to prevent**, so leaving one
    /// socket kind without a conversion does not leave a gap, it leaves a
    /// trap.
    ///
    /// Found by the agent that added `Socket::Udp` and deliberately left out
    /// of that change as out of scope — which was the right call, and this is
    /// the other half of it.
    ///
    /// # It is not a *listener*, and the name says so
    ///
    /// TCP and Unix hand back something you call `accept` on. UDP has no
    /// accept and no connection: a datagram service reads from the socket
    /// itself and replies to whoever the datagram came from. The supervisor
    /// binds it and sets no options (`cmd/rivet-sysd`'s `open_listeners`), so
    /// what arrives here is a plain bound socket and nothing more.
    #[cfg(unix)]
    #[must_use]
    pub fn into_udp_socket(self) -> Option<std::net::UdpSocket> {
        use std::os::fd::FromRawFd as _;
        if !self.adoptable {
            return None;
        }
        if !claim(self.fd) {
            return None;
        }
        // SAFETY: as `into_tcp_listener` — an owned, bound descriptor from the
        // supervisor, `self.adoptable` vouches for where it came from, and
        // `claim` has just made this its only owner.
        #[allow(unsafe_code)]
        Some(unsafe { std::net::UdpSocket::from_raw_fd(self.fd) })
    }
}

/// Every socket this application was handed, in the manifest's order.
///
/// Empty when none were passed, which is the ordinary state of an application
/// that declared no `listen`. **Also empty when `LISTEN_PID` names another
/// process** — see [`LISTEN_PID_ENV`].
///
/// # This does not clear the environment
///
/// systemd's `sd_listen_fds` takes an `unset_environment` flag; this does not,
/// because doing it silently is how a second caller gets an empty list and
/// concludes it was passed nothing. Call it once and keep the result.
#[must_use]
pub fn sockets() -> Vec<Socket> {
    let this = std::process::id();
    // **Not routed through `from_parts`.** That function is public and safe
    // and hands back sockets nobody may adopt (see its own doc) — this is the
    // one caller reading what the supervisor actually passed, so it is the
    // one caller allowed to mark what it builds adoptable.
    decide(
        std::env::var(LISTEN_PID_ENV).ok().as_deref(),
        std::env::var(LISTEN_FDS_ENV).ok().as_deref(),
        std::env::var(LISTEN_FDNAMES_ENV).ok().as_deref(),
        this,
        true,
    )
}

/// The decision, separated from the environment so every branch is testable.
///
/// `listen_pid` of `None` is treated as *for us*: the supervisor sets it, and a
/// program run by hand for development has no parent claiming the descriptors.
/// A pid that is set and does **not** match is treated as *not ours*, which is
/// the case that matters.
///
/// # What this returns cannot be adopted
///
/// **A `Socket` this function builds always answers `None` from
/// [`Socket::into_tcp_listener`] and the other `into_*` conversions.** This
/// function is public and safe, and takes whatever count and names it is
/// given — it has no way to know whether a descriptor at that number was ever
/// opened, let alone opened by this process's supervisor, so treating its
/// output as ownable would let safe code construct two owners of one
/// descriptor merely by naming a number twice. Only [`sockets`] and
/// [`socket`], which read this process's real environment, hand back a
/// `Socket` that may be adopted.
///
/// This is still the right place to test the decision above: which
/// descriptors exist and what they are named does not depend on whether they
/// may later be adopted.
#[must_use]
pub fn from_parts(
    listen_pid: Option<&str>,
    listen_fds: Option<&str>,
    listen_fdnames: Option<&str>,
    this_pid: u32,
) -> Vec<Socket> {
    decide(listen_pid, listen_fds, listen_fdnames, this_pid, false)
}

/// The shared logic behind [`from_parts`] and [`sockets`], parameterised on
/// whether the result may be adopted — see [`from_parts`]'s own doc for why
/// that can never be the caller's to choose.
fn decide(
    listen_pid: Option<&str>,
    listen_fds: Option<&str>,
    listen_fdnames: Option<&str>,
    this_pid: u32,
    adoptable: bool,
) -> Vec<Socket> {
    if let Some(pid) = listen_pid {
        match pid.trim().parse::<u32>() {
            Ok(claimed) if claimed == this_pid => {}
            // Set and not ours, or set and unparseable. Both mean the same
            // thing: this process cannot show that the descriptors are its own,
            // and acting on a descriptor that belongs to a parent is worse than
            // starting with none.
            _ => return Vec::new(),
        }
    }

    let Some(count) = listen_fds.and_then(|n| n.trim().parse::<i32>().ok()) else {
        return Vec::new();
    };
    if count <= 0 {
        return Vec::new();
    }

    // Fewer names than descriptors is not fatal: the descriptor is the thing
    // that works, and a missing name only costs the caller a lookup by
    // position. More names than descriptors is the supervisor's defect and the
    // extras are ignored rather than invented into sockets.
    let names: Vec<&str> = listen_fdnames.map(|n| n.split(':').collect()).unwrap_or_default();

    (0..count)
        .map(|index| Socket {
            fd: LISTEN_FDS_START + index,
            name: names
                .get(usize::try_from(index).unwrap_or(usize::MAX))
                .unwrap_or(&"")
                .to_string(),
            adoptable,
        })
        .collect()
}

/// The socket the manifest called `name`.
#[must_use]
pub fn socket(name: &str) -> Option<Socket> {
    sockets().into_iter().find(|s| s.name == name)
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn descriptors_start_at_three_and_are_named_in_order() {
        let got = from_parts(Some("42"), Some("2"), Some("tcp-8080:api"), 42);
        assert_eq!(
            got,
            vec![
                Socket { fd: 3, name: "tcp-8080".to_string(), adoptable: false },
                Socket { fd: 4, name: "api".to_string(), adoptable: false },
            ]
        );
    }

    /// The case the variable exists for: `LISTEN_PID` survives `exec`, so a
    /// child would otherwise act on its parent's descriptors.
    #[test]
    fn descriptors_claimed_by_another_process_are_not_ours() {
        assert!(from_parts(Some("42"), Some("2"), Some("a:b"), 99).is_empty());
    }

    #[test]
    fn an_unparseable_pid_is_treated_as_not_ours() {
        assert!(from_parts(Some("not-a-pid"), Some("1"), None, 42).is_empty());
    }

    /// A program run by hand has no supervisor setting the variable.
    #[test]
    fn an_absent_pid_is_treated_as_for_us() {
        assert_eq!(from_parts(None, Some("1"), Some("api"), 42).len(), 1);
    }

    #[test]
    fn declaring_no_listen_hands_over_nothing() {
        assert!(from_parts(Some("42"), None, None, 42).is_empty());
        assert!(from_parts(Some("42"), Some("0"), None, 42).is_empty());
    }

    /// The descriptor is what works; a missing name costs a lookup, not a
    /// socket.
    #[test]
    fn fewer_names_than_descriptors_still_yields_every_descriptor() {
        let got = from_parts(Some("42"), Some("3"), Some("only-one"), 42);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].name, "only-one");
        assert!(got[1].name.is_empty());
        assert!(got[2].name.is_empty());
    }

    #[test]
    fn more_names_than_descriptors_does_not_invent_a_socket() {
        let got = from_parts(Some("42"), Some("1"), Some("a:b:c"), 42);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "a");
    }

    #[test]
    fn a_tcp_socket_is_recognised_by_the_supervisors_naming_convention() {
        let got = from_parts(Some("42"), Some("2"), Some("tcp-8080:api"), 42);
        assert!(got[0].is_tcp(), "tcp-8080 should read as TCP");
        assert!(!got[1].is_tcp(), "a declared unix name should not");
    }

    /// **A descriptor can be owned once, and the second attempt is refused.**
    ///
    /// `PLAN.md` §123, Wave 5.3. Two owners means two closes, and on a kiosk
    /// application the descriptor after the last listener is the display
    /// channel — so the second drop takes the screen away from a program that
    /// is still running.
    ///
    /// Driven through `claim` rather than through `into_tcp_listener`, because
    /// the latter would need a real listening socket and this is a test about
    /// the bookkeeping. The conversion's only extra step is the `from_raw_fd`
    /// that `claim` is there to gate.
    #[test]
    fn a_descriptor_can_be_claimed_once_and_the_second_time_is_refused() {
        // Numbers no other test uses, because `CLAIMED` is process-wide — the
        // same reason the supervisor's connection-slot test is one test.
        assert!(claim(40), "the first claim must succeed");
        assert!(!claim(40), "the second claim on one descriptor must be refused");
        assert!(claim(41), "a different descriptor is unaffected");
        assert!(!claim(41));
    }

    /// A descriptor past the bitmask is **unclaimable**, not claimed. Refusing
    /// to hand out an owner is recoverable; handing out two is not.
    #[test]
    fn a_descriptor_the_guard_cannot_track_is_refused_rather_than_allowed() {
        assert!(!claim(64), "64 is past the mask and must not be handed out");
        assert!(!claim(1_000_000));
        assert!(!claim(-1), "a negative descriptor is not a descriptor");
    }

    /// **Reading the list stays free**, which is the half the audit's own
    /// proposal would have broken: `examples/aura/aura-renderer` calls
    /// `listen::socket(name)` once per slot, and a take-once on `sockets()`
    /// would have answered `None` for three of its four addons with nothing
    /// saying why.
    #[test]
    fn the_list_can_be_read_as_many_times_as_an_application_likes() {
        let once = from_parts(Some("42"), Some("2"), Some("a:b"), 42);
        let twice = from_parts(Some("42"), Some("2"), Some("a:b"), 42);
        assert_eq!(once, twice, "reading the list is not taking anything");
        assert_eq!(once.len(), 2);
    }

    /// **A UDP socket is reachable by name from this side too** — the half
    /// that makes `Socket::Udp` usable rather than merely deliverable.
    ///
    /// `cmd/rivet-sysd` names a UDP descriptor `udp-<port>` in
    /// `LISTEN_FDNAMES`, exactly as it names a TCP one `tcp-<port>`. If this
    /// side could not find it by that name, the supervisor would be binding a
    /// socket no application could locate, and the failure would be an
    /// application that starts, reports healthy and never answers.
    ///
    /// Asserted through `from_parts` rather than through `into_udp_socket`,
    /// for the reason
    /// [`a_descriptor_can_be_claimed_once_and_the_second_time_is_refused`]
    /// gives: the conversion needs a real bound socket and this is a test
    /// about the lookup.
    #[test]
    fn a_udp_socket_is_found_by_the_name_the_supervisor_gives_it() {
        let got = from_parts(Some("42"), Some("2"), Some("udp-53:tcp-80"), 42);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].name, "udp-53");
        assert_eq!(got[1].name, "tcp-80");
    }

    /// `ADR-015` §3.2 writes `tcp:80`; the wire form must not, or it arrives as
    /// two names. This is that rule stated as a test on the consumer side.
    #[test]
    fn a_name_never_contains_the_separator() {
        let got = from_parts(Some("42"), Some("2"), Some("tcp-80:api"), 42);
        for socket in got {
            assert!(
                !socket.name.contains(':'),
                "a name carrying the separator would have split into two"
            );
        }
    }

    /// **A `Socket` built by the public `from_parts` cannot adopt a real,
    /// already-owned listener merely by naming its descriptor.**
    ///
    /// A real `TcpListener` is bound so this has a live descriptor `d` to name.
    /// `from_parts` is then asked for a count that reaches `d`, exactly as
    /// unsanitised strings could reach it — and the `Socket` it builds for `d`
    /// must refuse to become a second owner. Because `into_tcp_listener` checks
    /// `adoptable` before it ever calls `claim`, this never touches the
    /// process-wide claim bitmask, so it cannot disturb the `claim`-driven
    /// tests elsewhere in this module regardless of which descriptor number the
    /// OS happens to hand back.
    #[cfg(unix)]
    #[test]
    fn from_parts_cannot_adopt_a_real_listeners_descriptor() {
        use std::os::fd::AsRawFd as _;

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a real listener");
        let d = listener.as_raw_fd();
        assert!(d >= LISTEN_FDS_START, "stdio occupies 0..3, so a real fd is at least 3");

        // A count that reaches `d`: the descriptors `from_parts` invents run
        // `3..3+count`, so the last one lands on `d`.
        let count = (d - LISTEN_FDS_START + 1).to_string();
        let mut got = from_parts(Some("42"), Some(&count), None, 42);
        let described = got.pop().expect("at least one socket");
        assert_eq!(described.fd(), d, "the last socket must describe `d`");

        assert!(
            described.into_tcp_listener().is_none(),
            "a Socket built by from_parts must never adopt a descriptor"
        );

        // The original listener is still the only owner: nothing above ever
        // called `from_raw_fd`, so a real connection still completes.
        let addr = listener.local_addr().expect("a local address");
        let client = std::net::TcpStream::connect(addr).expect("a client can still connect");
        let (_accepted, _) = listener.accept().expect("the original listener still accepts");
        drop(client);
    }

    /// The unix conversion refuses a `from_parts` socket the same way the TCP
    /// one does — no real socket is needed, since `adoptable` is checked before
    /// the descriptor is looked at at all.
    #[cfg(unix)]
    #[test]
    fn from_parts_cannot_become_a_unix_listener() {
        let got = from_parts(Some("42"), Some("1"), Some("api"), 42);
        let described = got.into_iter().next().expect("one socket");
        assert!(described.into_unix_listener().is_none());
    }

    /// The UDP conversion refuses a `from_parts` socket the same way.
    #[cfg(unix)]
    #[test]
    fn from_parts_cannot_become_a_udp_socket() {
        let got = from_parts(Some("42"), Some("1"), Some("udp-53"), 42);
        let described = got.into_iter().next().expect("one socket");
        assert!(described.into_udp_socket().is_none());
    }

    /// **The environment path still marks what it builds adoptable.**
    ///
    /// Direct field access rather than a real conversion, because the
    /// SINGLE-USE half of "adopts exactly once" is already
    /// `a_descriptor_can_be_claimed_once_and_the_second_time_is_refused`'s own
    /// job, and that test is unchanged by this fix. What this finding's repair
    /// could have broken is the OTHER half — that the environment path still
    /// produces something adoptable at all, now that a second source
    /// ([`from_parts`]) deliberately never does.
    ///
    /// Built through the private `decide` helper with `adoptable: true` —
    /// what `sockets()` passes when it reads the real environment — rather
    /// than through real environment variables, which are process-wide and a
    /// parallel test could be reading or writing them.
    ///
    /// **Not driven through `into_tcp_listener` on a real descriptor**, for
    /// the reason `a_descriptor_can_be_claimed_once_and_the_second_time_is_refused`
    /// already gives: that needs a real listening socket, and the bitmask
    /// `claim` guards only covers descriptors below 64 — a real one a busy
    /// test binary hands out is not guaranteed to land under that, and a test
    /// that could fail for a reason unrelated to what it is testing is worse
    /// than one that tests less.
    #[test]
    fn the_environment_path_still_marks_its_sockets_adoptable() {
        let got = decide(Some("42"), Some("1"), Some("api"), 42, true);
        assert!(
            got[0].adoptable,
            "sockets() must still be able to adopt what it hands out"
        );
    }
}
