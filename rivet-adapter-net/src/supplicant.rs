//! **Driving `wpa_supplicant` over its control socket** — `ADR-004` §4.
//!
//! > `wpa_supplicant` is a daemon, not a library. Rivet therefore talks to it
//! > over its control socket rather than linking it. … This is the cleanest
//! > boundary available and it satisfies `ADR-001` section 5 exactly: no
//! > `unsafe`, no ABI coupling, no `bindgen`, no C in the Rust build graph, and
//! > independent lifecycle and restart.
//!
//! §4 sketched the exchange and left the client to implementation. This is it.
//!
//! # There is no `wpa_supplicant.conf`, and that is the ruling
//!
//! `ADR-004` §5: *"`wpa_supplicant.conf` does not exist on a Rivet system.
//! Networks are configured in Rivet's own configuration, in Rivet's own format,
//! and the supplicant is driven at runtime. Its configuration file is an
//! implementation detail that has been removed rather than generated."*
//!
//! So the supplicant is started with **no configuration at all** — only a
//! control-interface directory — and every network it knows about is one this
//! module told it about after it was running. That is why `ADD_NETWORK` and
//! `SET_NETWORK` appear here rather than a file writer.
//!
//! # The one detail that decides whether association works
//!
//! `SET_NETWORK <id> psk` takes **either** a 64-character hex PMK *unquoted*
//! **or** a passphrase *in quotes*, and it does completely different things
//! with them. Rivet always sends the hex form, because `ADR-004` §5 keeps the
//! passphrase off the machine entirely — `rivet_crypto::pmk` derives it once,
//! somewhere a person typed it, and only the key travels.
//!
//! Sending a quoted passphrase would make the supplicant derive again, which
//! would work and would mean a plaintext credential had reached the board. The
//! quoting is the whole difference and it is one character.

use rivet_core::ssid::Ssid;

/// Where the supplicant is asked to put its control sockets.
///
/// **`/run` and not `/state`**: a control socket is true only while the process
/// is, and `rivet_core::mount` already makes `/run` a tmpfs. One directory,
/// named here and nowhere else (`ADR-011`).
pub const CONTROL_DIR: &str = "/run/wpa_supplicant";

/// The driver `wpa_supplicant` is told to use.
///
/// `nl80211` is the kernel's modern interface and the only one Buildroot builds
/// this supplicant with — `tools/buildroot/configs/rivet_aarch64_defconfig` sets
/// `BR2_PACKAGE_WPA_SUPPLICANT_NL80211` and leaves `WEXT`, which is deprecated,
/// off. Naming it explicitly rather than letting the supplicant guess means a
/// build that lost the option fails loudly here instead of silently choosing
/// nothing.
pub const DRIVER: &str = "nl80211";

/// Where the image puts the supplicant. Spelled here and in `cmd/rivet-image`'s
/// `SUPPLICANT_PATH`, the same convention every other program Rivet execs has.
pub const MECHANISM: &str = "/system/bin/wpa_supplicant";

/// A conversation with a running supplicant.
///
/// **A port, so the exchange can be driven without a radio.** Every interesting
/// thing here is a sequence — the order of `ADD_NETWORK`, `SET_NETWORK` and
/// `ENABLE_NETWORK`, and what happens when one of them answers `FAIL` — and
/// none of it is testable against real hardware in a way that could be run.
pub trait Control {
    /// Send one command and return what came back, with the trailing newline
    /// removed.
    ///
    /// # Errors
    ///
    /// Only a transport failure — the socket being gone, or a timeout. A
    /// supplicant that answers `FAIL` has answered, and that is `Ok`. **The
    /// error string must never include `command`**: some commands carry key
    /// material, and a caller that wants a safe label for its diagnostic
    /// attaches one of its own rather than this layer echoing back what it
    /// was asked.
    fn request(&mut self, command: &str) -> Result<String, String>;
}

/// Why association did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupplicantError {
    /// The control socket could not be reached at all.
    Unreachable { detail: String },
    /// A command was refused by the supplicant.
    Refused { command: &'static str, answer: String },
    /// `ADD_NETWORK` answered something that is not a network id.
    NotANetworkId { answer: String },
    /// It never reported itself associated.
    NeverCompleted { seconds: u64, last: String },
}

impl core::fmt::Display for SupplicantError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unreachable { detail } => write!(
                f,
                "the supplicant's control socket at {CONTROL_DIR} could not be reached: {detail}. \
                 That is this machine's own process rather than the network — check that \
                 {MECHANISM} is on this card and that it started"
            ),
            Self::Refused { command, answer } => write!(
                f,
                "the supplicant refused `{command}` with {answer:?}. NOTHING WAS JOINED"
            ),
            Self::NotANetworkId { answer } => write!(
                f,
                "ADD_NETWORK answered {answer:?}, which is not a network id. Nothing was joined"
            ),
            Self::NeverCompleted { seconds, last } => write!(
                f,
                "the supplicant did not report itself associated within {seconds}s; its last \
                 state was {last:?}. The credential, the network's presence and its signal are \
                 all candidates, and `wpa_cli status` on the board distinguishes them"
            ),
        }
    }
}

/// What the supplicant says about itself, reduced to what matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Associated, keys installed, ready for an address.
    Completed,
    /// Working on it — scanning, authenticating, handshaking.
    Working,
    /// Not associated and not trying in a way that will succeed.
    Idle,
}

impl State {
    /// Read `wpa_state=` out of a `STATUS` reply.
    ///
    /// **Only `COMPLETED` means joined.** `ASSOCIATED` is not enough: it is the
    /// state between association and the four-way handshake, and a wrong
    /// passphrase reaches it before failing. Treating it as success would
    /// produce a machine that reports a network it cannot pass traffic on.
    #[must_use]
    pub fn from_status(reply: &str) -> Self {
        let state = reply
            .lines()
            .find_map(|line| line.strip_prefix("wpa_state="))
            .unwrap_or("")
            .trim();
        match state {
            "COMPLETED" => Self::Completed,
            "SCANNING" | "AUTHENTICATING" | "ASSOCIATING" | "ASSOCIATED" | "4WAY_HANDSHAKE"
            | "GROUP_HANDSHAKE" => Self::Working,
            _ => Self::Idle,
        }
    }
}

/// Whether a `STATUS` reply that reports `COMPLETED` is naming the network
/// this call asked for, rather than some other network that was already
/// associated before this call began. Matches on `id=` or `ssid=`, exactly,
/// either one being enough.
///
/// **Necessary, and not implied by `wpa_state=COMPLETED` alone.** `rivet-netd`
/// reuses a running supplicant across a reload, so a machine already joined to
/// one network can go on reporting `COMPLETED` for it while a different
/// network is mid-request. Without this check, that stale success would be
/// handed back at once as though the new request had succeeded.
fn completed_network_is(reply: &str, id: &str, ssid: &Ssid) -> bool {
    reply
        .lines()
        .any(|line| line.strip_prefix("id=").is_some_and(|v| v.trim() == id))
        || reply
            .lines()
            .any(|line| line.strip_prefix("ssid=").is_some_and(|v| v.trim() == ssid.as_str()))
}

/// Tell a running supplicant to join one network, and wait until it has.
///
/// # The order is the whole of it
///
/// `ADD_NETWORK` first, because everything after it names the id it returns;
/// the SSID and the key before `ENABLE_NETWORK`, because enabling a network
/// with neither makes the supplicant start scanning for something it cannot
/// join and report failures that describe nothing.
///
/// # Errors
///
/// [`SupplicantError`], every variant of which means **nothing was joined**.
pub fn associate(
    control: &mut dyn Control,
    ssid: &Ssid,
    pmk_hex: &str,
    attempts: u32,
    sleep: &mut dyn FnMut(),
) -> Result<(), SupplicantError> {
    let ask = |control: &mut dyn Control, command: &str, label: &'static str| {
        control
            .request(command)
            .map_err(|detail| SupplicantError::Unreachable {
                detail: format!("{label}: {detail}"),
            })
            .and_then(|answer| {
                if answer.trim() == "FAIL" || answer.trim().is_empty() {
                    Err(SupplicantError::Refused {
                        command: label,
                        answer,
                    })
                } else {
                    Ok(answer)
                }
            })
    };

    let id = ask(control, "ADD_NETWORK", "ADD_NETWORK")?;
    let id = id.trim();
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err(SupplicantError::NotANetworkId {
            answer: id.to_owned(),
        });
    }

    // **The SSID is QUOTED and the key is NOT.** That difference is the whole
    // of `SET_NETWORK`'s type system: a quoted `psk` is a passphrase the
    // supplicant would derive from, and an unquoted one is the 32-byte key
    // itself. Rivet only ever has the key — `ADR-004` §5.
    ask(
        control,
        &format!("SET_NETWORK {id} ssid \"{}\"", ssid.as_str()),
        "SET_NETWORK ssid",
    )?;
    ask(
        control,
        &format!("SET_NETWORK {id} psk {pmk_hex}"),
        "SET_NETWORK psk",
    )?;
    ask(control, &format!("ENABLE_NETWORK {id}"), "ENABLE_NETWORK")?;

    // **Polled rather than waited on with events.** `ATTACH` would deliver
    // `CTRL-EVENT-CONNECTED` asynchronously and would need a second socket kept
    // open for the life of the wait; `STATUS` answers the same question with
    // one request and no state. The cost is latency measured in the poll
    // interval, on something that takes seconds anyway.
    let mut last = String::new();
    // **Sent at most once.** A supplicant `rivet-netd` reused can already be
    // `COMPLETED` on a previous network; discovering that is not a reason to
    // ask it to switch again on every later poll, since the answer would not
    // change and it would only add traffic on a socket already being polled.
    let mut reselected = false;
    for attempt in 0..attempts {
        let status = control
            .request("STATUS")
            .map_err(|detail| SupplicantError::Unreachable {
                detail: format!("STATUS: {detail}"),
            })?;
        match State::from_status(&status) {
            State::Completed if completed_network_is(&status, id, ssid) => return Ok(()),
            State::Completed => {
                // **Somebody else's success.** `wpa_state=COMPLETED` alone
                // does not say the network that just completed is the one
                // this call asked for — see `completed_network_is`. Ask the
                // supplicant to switch to the one that is.
                last = "COMPLETED, but not the requested network".to_owned();
                if !reselected {
                    ask(control, &format!("SELECT_NETWORK {id}"), "SELECT_NETWORK")?;
                    reselected = true;
                }
            }
            other => last = format!("{other:?}"),
        }
        if attempt + 1 < attempts {
            sleep();
        }
    }
    Err(SupplicantError::NeverCompleted {
        seconds: u64::from(attempts),
        last,
    })
}

/// The arguments the supplicant is started with.
///
/// **No `-c`, which is the ruling rather than an omission** — `ADR-004` §5
/// removes the configuration file rather than generating one. `-C` gives it a
/// control-interface directory and nothing else, so it comes up knowing no
/// networks at all and [`associate`] is the only thing that ever tells it one.
///
/// **And no `-B`.** That flag makes the supplicant fork and the parent exit.
/// The supplicant is a service `cmd/rivet-sysd` starts and keeps running, one
/// per wireless interface the document names, so the process the supervisor
/// started has to be the one that stays: a double fork would leave it holding
/// a parent that exited and lose the exit status it restarts on.
///
/// `rivet-netd` does not start it. It waits for the control socket under
/// [`CONTROL_DIR`] and [`associate`]s over it; and because the network the
/// supplicant knows lives only in its memory, the supervisor runs `rivet-netd`
/// again whenever it restarts a supplicant.
#[must_use]
pub fn arguments(interface: &str) -> Vec<String> {
    vec![
        "-i".to_owned(),
        interface.to_owned(),
        "-D".to_owned(),
        DRIVER.to_owned(),
        "-C".to_owned(),
        CONTROL_DIR.to_owned(),
    ]
}

/// **Unix only.** The rest of this module is pure and is tested on the
/// development host; a `AF_UNIX` datagram socket is not a thing Windows has, and
/// `ADR-009` §3.6.1's rule is that the workspace builds on the development host.
/// Gating the socket rather than the module is what keeps [`associate`] — where
/// all the judgement is — testable there.
#[cfg(unix)]
/// The real control socket.
///
/// # It is a DATAGRAM socket, and that is the part that surprises people
///
/// `wpa_supplicant`'s control interface is `SOCK_DGRAM` over an `AF_UNIX` path,
/// not a stream. A datagram socket has no connection to accept and no framing
/// to parse — **one send is one command and one receive is one whole reply** —
/// but it does mean this end needs a bound address of its own for the answer to
/// come back to, which a stream client would not.
///
/// So each client binds a socket in the same directory, named after itself and
/// its pid, and removes it on drop. A stale one left by a killed process would
/// make the next bind fail with `EADDRINUSE` on a machine that has no way to
/// clean it up.
///
/// # Unsolicited events arrive on the same socket
///
/// The supplicant sends `<3>CTRL-EVENT-…` lines to any client that has
/// `ATTACH`ed, and this one deliberately does not attach — see [`associate`],
/// which polls `STATUS` instead. Events can still arrive from a previous
/// attach, so a reply beginning with `<` is skipped rather than returned: a
/// caller that took `<3>CTRL-EVENT-SCAN-STARTED` for the answer to
/// `ADD_NETWORK` would go on to configure a network id that does not exist.
pub struct ControlSocket {
    socket: std::os::unix::net::UnixDatagram,
    /// This end's own path, removed on drop.
    ours: std::path::PathBuf,
}

#[cfg(unix)]
impl ControlSocket {
    /// Open a conversation with the supplicant for one interface.
    ///
    /// # Errors
    ///
    /// The directory not existing, the supplicant's socket not being there, or
    /// this end failing to bind. All three mean the supplicant is not running
    /// or not ready, which is a different thing from a network that will not
    /// join.
    pub fn open(interface: &str) -> Result<Self, String> {
        Self::open_in(std::path::Path::new(CONTROL_DIR), interface)
    }

    /// As [`open`](Self::open), against an arbitrary directory rather than
    /// the hard-coded [`CONTROL_DIR`]. `CONTROL_DIR` is `/run/wpa_supplicant`,
    /// which only root can write to, so this is also how a test drives the
    /// real socket against a directory it owns.
    fn open_in(dir: &std::path::Path, interface: &str) -> Result<Self, String> {
        let theirs = dir.join(interface);
        // Named after the pid so two clients cannot collide, and because a
        // fixed name left behind by a killed process would make every later
        // open fail.
        let ours = dir.join(format!("rivet-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&ours);

        let socket = std::os::unix::net::UnixDatagram::bind(&ours)
            .map_err(|e| format!("cannot bind {}: {e}", ours.display()))?;
        socket
            .connect(&theirs)
            .map_err(|e| format!("cannot reach {}: {e}", theirs.display()))?;
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(REPLY_TIMEOUT_SECONDS)))
            .map_err(|e| format!("cannot set a read timeout: {e}"))?;
        Ok(Self { socket, ours })
    }
}

/// How long to wait for one reply.
///
/// The supplicant answers a control command immediately — it is a local socket
/// and the work is a table lookup. This exists so that a supplicant which has
/// wedged cannot hold a boot open, not because any reply is slow.
#[cfg(unix)]
const REPLY_TIMEOUT_SECONDS: u64 = 5;

/// The largest reply this reads. `STATUS` is the longest and is a few hundred
/// bytes; the supplicant's own control buffer is 4096.
#[cfg(unix)]
const REPLY_MAX: usize = 4096;

#[cfg(unix)]
impl Drop for ControlSocket {
    fn drop(&mut self) {
        // **Removed, because nothing else would.** `/run` is a tmpfs so a
        // reboot clears it, but `rivet-netd` runs many times per boot and a
        // socket per run would accumulate.
        let _ = std::fs::remove_file(&self.ours);
    }
}

#[cfg(unix)]
impl Control for ControlSocket {
    fn request(&mut self, command: &str) -> Result<String, String> {
        // **`command` itself never appears below.** Some commands carry key
        // material (`SET_NETWORK … psk …`), and this is the layer that would
        // leak it into a journal — see the `Control::request` contract. A
        // caller that knows a safe name for what it asked attaches its own
        // label to whichever of these strings comes back.
        self.socket
            .send(command.as_bytes())
            .map_err(|e| format!("cannot send: {e}"))?;

        // Unsolicited events share this socket. Skip them rather than return
        // one as an answer; bounded, so a supplicant producing nothing but
        // events cannot hold this open forever.
        for _ in 0..EVENTS_SKIPPED {
            let mut buffer = vec![0u8; REPLY_MAX];
            let read = self
                .socket
                .recv(&mut buffer)
                .map_err(|e| format!("no reply: {e}"))?;
            buffer.truncate(read);
            let reply = String::from_utf8_lossy(&buffer).trim_end().to_string();
            if !reply.starts_with('<') {
                return Ok(reply);
            }
        }
        Err(format!(
            "only unsolicited events came back, {EVENTS_SKIPPED} of them"
        ))
    }
}

/// How many event messages to step over while looking for a reply.
#[cfg(unix)]
const EVENTS_SKIPPED: u32 = 16;

#[cfg(test)]
#[path = "supplicant_tests.rs"]
mod tests;
