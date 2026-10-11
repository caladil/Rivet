//! The exchange, driven without a radio.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

/// A supplicant that answers as told, and remembers what it was asked.
struct Fake {
    said: Vec<String>,
    /// Replies for anything that is not `STATUS`, in order.
    replies: Vec<String>,
    /// What `STATUS` reports, one per call; the last repeats.
    states: Vec<&'static str>,
    status_calls: usize,
    /// Fail the transport entirely.
    unreachable: bool,
}

impl Fake {
    fn joining() -> Self {
        Self {
            said: Vec::new(),
            replies: vec!["0".into(), "OK".into(), "OK".into(), "OK".into()],
            states: vec!["COMPLETED"],
            status_calls: 0,
            unreachable: false,
        }
    }
}

impl Control for Fake {
    fn request(&mut self, command: &str) -> Result<String, String> {
        if self.unreachable {
            return Err("No such file or directory".to_string());
        }
        self.said.push(command.to_string());
        if command == "STATUS" {
            let i = self.status_calls.min(self.states.len() - 1);
            self.status_calls += 1;
            // `ssid=` names the network this fake is pretending to run,
            // which is always the one `join`/`associate` was asked for in
            // these tests — a completed status naming some OTHER network is
            // its own coverage, below.
            return Ok(format!(
                "bssid=00:00:00:00:00:00\nssid=an-example-network\nwpa_state={}\n",
                self.states[i]
            ));
        }
        if self.replies.is_empty() {
            return Ok("OK".to_string());
        }
        Ok(self.replies.remove(0))
    }
}

fn ssid() -> Ssid {
    Ssid::new("an-example-network").expect("valid")
}

/// A PMK-shaped value. `AGENTS.md` §8.3 — a fixture, not a key to anything.
const NOT_A_REAL_KEY: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

fn join(fake: &mut Fake) -> Result<(), SupplicantError> {
    associate(fake, &ssid(), NOT_A_REAL_KEY, 5, &mut || {})
}

/// **The order is the whole of it.** Everything after `ADD_NETWORK` names the
/// id it returned, and `ENABLE_NETWORK` must come after the SSID and the key —
/// enabling a network with neither makes the supplicant scan for something it
/// cannot join and report failures that describe nothing.
#[test]
fn the_conversation_happens_in_the_order_that_works() {
    let mut fake = Fake::joining();
    join(&mut fake).expect("it joins");

    let asked: Vec<&str> = fake.said.iter().map(String::as_str).collect();
    assert_eq!(asked[0], "ADD_NETWORK");
    assert!(asked[1].starts_with("SET_NETWORK 0 ssid "), "{asked:?}");
    assert!(asked[2].starts_with("SET_NETWORK 0 psk "), "{asked:?}");
    assert_eq!(asked[3], "ENABLE_NETWORK 0");
    assert_eq!(asked[4], "STATUS");
}

/// **The SSID is quoted and the key is not, and that difference is the whole of
/// `SET_NETWORK`'s type system.** A quoted `psk` is a PASSPHRASE the supplicant
/// derives from again; an unquoted one is the key itself. Rivet only ever has
/// the key, and sending it quoted would associate — with a different key.
#[test]
fn the_ssid_is_quoted_and_the_key_is_not() {
    let mut fake = Fake::joining();
    join(&mut fake).expect("joins");
    let ssid_cmd = fake.said.iter().find(|c| c.contains(" ssid ")).expect("ssid");
    let psk_cmd = fake.said.iter().find(|c| c.contains(" psk ")).expect("psk");

    assert_eq!(ssid_cmd, "SET_NETWORK 0 ssid \"an-example-network\"");
    assert_eq!(
        psk_cmd,
        &format!("SET_NETWORK 0 psk {NOT_A_REAL_KEY}"),
        "the key was quoted, which makes the supplicant DERIVE from it"
    );
    assert!(!psk_cmd.contains('"'), "the key must not be quoted: {psk_cmd}");
}

/// The id `ADD_NETWORK` returns is the one used, not an assumed zero.
#[test]
fn the_network_id_the_supplicant_chose_is_the_one_used() {
    let mut fake = Fake::joining();
    fake.replies[0] = "3".to_string();
    join(&mut fake).expect("joins");
    assert!(
        fake.said.iter().any(|c| c == "ENABLE_NETWORK 3"),
        "{:?}",
        fake.said
    );
}

/// **Only `COMPLETED` is joined.** `ASSOCIATED` is the state between
/// association and the four-way handshake, and a WRONG PASSPHRASE reaches it
/// before failing. Treating it as success produces a machine that reports a
/// network it cannot pass traffic on.
#[test]
fn associated_is_not_completed() {
    assert_eq!(State::from_status("wpa_state=COMPLETED\n"), State::Completed);
    for working in [
        "SCANNING",
        "AUTHENTICATING",
        "ASSOCIATING",
        "ASSOCIATED",
        "4WAY_HANDSHAKE",
        "GROUP_HANDSHAKE",
    ] {
        assert_eq!(
            State::from_status(&format!("wpa_state={working}\n")),
            State::Working,
            "{working} was taken for joined"
        );
    }
    assert_eq!(State::from_status("wpa_state=INACTIVE\n"), State::Idle);
    assert_eq!(State::from_status("wpa_state=DISCONNECTED\n"), State::Idle);
    // A reply with no state at all is not a success.
    assert_eq!(State::from_status("bssid=x\n"), State::Idle);
}

/// A supplicant that works its way there is waited for.
#[test]
fn a_supplicant_that_takes_a_few_seconds_is_waited_for() {
    let mut fake = Fake::joining();
    fake.states = vec!["SCANNING", "ASSOCIATING", "4WAY_HANDSHAKE", "COMPLETED"];
    let mut slept = 0_u32;
    associate(&mut fake, &ssid(), NOT_A_REAL_KEY, 8, &mut || slept += 1).expect("joins");
    assert_eq!(slept, 3, "it should sleep between looks and not after the last");
}

/// **A wrong key looks like this**, and the refusal has to be diagnosable: the
/// supplicant never completes, and the message says which three things to
/// suspect.
#[test]
fn a_supplicant_that_never_completes_is_reported_with_its_last_state() {
    let mut fake = Fake::joining();
    fake.states = vec!["4WAY_HANDSHAKE"];
    let refused = associate(&mut fake, &ssid(), NOT_A_REAL_KEY, 3, &mut || {})
        .expect_err("it never completes");
    let said = refused.to_string();
    assert!(said.contains("did not report itself associated"), "{said}");
    assert!(said.contains("Working"), "{said}");
}

/// A refused command stops the conversation rather than carrying on into
/// commands that name an id that was never created.
#[test]
fn a_refusal_stops_the_conversation() {
    let mut fake = Fake::joining();
    fake.replies[0] = "FAIL".to_string();
    let refused = join(&mut fake).expect_err("ADD_NETWORK refused");
    assert!(
        matches!(refused, SupplicantError::Refused { command: "ADD_NETWORK", .. }),
        "{refused:?}"
    );
    assert_eq!(fake.said.len(), 1, "it kept talking after a refusal");
}

#[test]
fn an_answer_that_is_not_a_network_id_is_refused() {
    let mut fake = Fake::joining();
    fake.replies[0] = "UNKNOWN COMMAND".to_string();
    let refused = join(&mut fake).expect_err("not an id");
    assert!(matches!(refused, SupplicantError::NotANetworkId { .. }), "{refused:?}");
}

/// **An unreachable socket is this machine's own fault, not the network's**,
/// and the message says so — it sends an operator to the supplicant process
/// rather than to the router.
#[test]
fn an_unreachable_socket_names_this_machine_rather_than_the_network() {
    let mut fake = Fake::joining();
    fake.unreachable = true;
    let refused = join(&mut fake).expect_err("unreachable");
    let said = refused.to_string();
    assert!(said.contains("this machine's own process"), "{said}");
    assert!(said.contains(MECHANISM), "{said}");
}

/// **No `-c` and no `-B`.** The first is `ADR-004` §5 removing the
/// configuration file; the second would daemonise, and a process that forks and
/// exits is one `rivet-sysd` cannot supervise.
#[test]
fn the_supplicant_is_started_with_no_config_and_stays_in_the_foreground() {
    let args = arguments("wlan0");
    assert!(!args.iter().any(|a| a == "-c"), "{args:?}");
    assert!(!args.iter().any(|a| a == "-B"), "{args:?}");
    assert!(args.contains(&"-i".to_string()) && args.contains(&"wlan0".to_string()));
    assert!(args.contains(&DRIVER.to_string()), "{args:?}");
    assert!(args.contains(&CONTROL_DIR.to_string()), "{args:?}");
}

/// A supplicant whose `STATUS` answer is scripted call by call, so a test can
/// simulate a stale association surviving into a new request — something
/// `Fake` cannot do, since it always answers `STATUS` with the network it was
/// just told about.
struct Scripted {
    said: Vec<String>,
    /// What `ADD_NETWORK` answers.
    add_network_id: &'static str,
    /// One full `STATUS` body per call, in order; the last repeats once the
    /// list is exhausted.
    status: Vec<&'static str>,
    status_calls: usize,
}

impl Control for Scripted {
    fn request(&mut self, command: &str) -> Result<String, String> {
        self.said.push(command.to_string());
        if command == "ADD_NETWORK" {
            return Ok(self.add_network_id.to_string());
        }
        if command == "STATUS" {
            let i = self.status_calls.min(self.status.len().saturating_sub(1));
            self.status_calls += 1;
            return Ok((*self.status.get(i).unwrap_or(&"wpa_state=UNKNOWN\n")).to_string());
        }
        Ok("OK".to_string())
    }
}

/// **A machine already joined to A, asked to join B, must not report success
/// for A.** `rivet-netd` reuses a running supplicant across a reload
/// ("Start the supplicant unless one is already there"), so a stale
/// `COMPLETED` for the previous network is exactly what a request for a new
/// one can see.
#[test]
fn a_stale_association_is_reselected_once_but_never_accepted_as_the_new_one() {
    let ssid_b = Ssid::new("B").expect("valid");
    let mut fake = Scripted {
        said: Vec::new(),
        add_network_id: "1",
        status: vec!["wpa_state=COMPLETED\nid=0\nssid=A\n"],
        status_calls: 0,
    };
    let refused = associate(&mut fake, &ssid_b, NOT_A_REAL_KEY, 3, &mut || {})
        .expect_err("A completing is not B completing");
    assert!(matches!(refused, SupplicantError::NeverCompleted { .. }), "{refused:?}");
    let selects = fake.said.iter().filter(|c| c.as_str() == "SELECT_NETWORK 1").count();
    assert_eq!(selects, 1, "SELECT_NETWORK must be sent exactly once: {:?}", fake.said);
    assert!(refused.to_string().contains("COMPLETED"), "{refused}");
}

/// The other half of the case above: once the supplicant actually completes
/// the newly selected network, that is success.
#[test]
fn switching_networks_succeeds_once_the_new_one_actually_completes() {
    let ssid_b = Ssid::new("B").expect("valid");
    let mut fake = Scripted {
        said: Vec::new(),
        add_network_id: "1",
        status: vec![
            "wpa_state=COMPLETED\nid=0\nssid=A\n",
            "wpa_state=COMPLETED\nid=1\nssid=B\n",
        ],
        status_calls: 0,
    };
    associate(&mut fake, &ssid_b, NOT_A_REAL_KEY, 3, &mut || {}).expect("B completes next");
    let selects = fake.said.iter().filter(|c| c.as_str() == "SELECT_NETWORK 1").count();
    assert_eq!(selects, 1, "{:?}", fake.said);
}

/// A reload that re-asks for the SAME network the supplicant is already on
/// must not wait for a reselect that was never needed.
#[test]
fn reloading_the_same_network_succeeds_at_once_without_reselecting() {
    let ssid_b = Ssid::new("B").expect("valid");
    let mut fake = Scripted {
        said: Vec::new(),
        add_network_id: "0",
        status: vec!["wpa_state=COMPLETED\nid=0\nssid=B\n"],
        status_calls: 0,
    };
    associate(&mut fake, &ssid_b, NOT_A_REAL_KEY, 3, &mut || {})
        .expect("already the requested network");
    assert!(
        !fake.said.iter().any(|c| c.starts_with("SELECT_NETWORK")),
        "{:?}",
        fake.said
    );
}

/// `COMPLETED` with neither an `id=` nor an `ssid=` line names nothing, so it
/// cannot be taken as this request succeeding.
#[test]
fn a_completed_status_naming_no_network_is_not_success() {
    let mut fake = Scripted {
        said: Vec::new(),
        add_network_id: "0",
        status: vec!["wpa_state=COMPLETED\n"],
        status_calls: 0,
    };
    let refused = associate(&mut fake, &ssid(), NOT_A_REAL_KEY, 2, &mut || {})
        .expect_err("nothing here identifies the requested network");
    assert!(matches!(refused, SupplicantError::NeverCompleted { .. }), "{refused:?}");
}

/// **This coverage lives against the real socket, not `Fake`.** `Fake`
/// returns an already-safe string on failure and can never reproduce a leak
/// that lives in `ControlSocket::request`'s own formatting — only the real
/// socket proves that is fixed. Unix only, for the reason `ControlSocket`
/// itself is.
#[cfg(unix)]
mod real_control_socket {
    use super::*;
    use std::os::unix::net::UnixDatagram;

    /// PMK-shaped and load-bearing only as a canary that must appear in no
    /// error. `AGENTS.md` §8.3 — a fixture, not a key to anything.
    const CANARY: &str = "c0ffee00c0ffee00c0ffee00c0ffee00c0ffee00c0ffee00c0ffee00c0ffee00";

    /// A scratch control directory that removes itself. Tagged with a
    /// per-process counter as well as the pid: `cargo test` runs many of
    /// these concurrently as threads of one process, so the pid alone
    /// repeats.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let at = std::env::temp_dir().join(format!(
                "rivet-adapter-net-supplicant-{tag}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&at);
            std::fs::create_dir_all(&at).expect("a scratch control directory");
            Self(at)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Binds a fake peer at `<dir>/<interface>`, opens the real
    /// `ControlSocket` against it, and queues the two replies `associate`
    /// needs before it ever reaches `SET_NETWORK psk` — the command that
    /// carries the key. A `UnixDatagram` queues a sent datagram for its peer
    /// whether or not that peer has called `recv` yet, so both replies can be
    /// queued up front: no second thread, and no race between a reply
    /// landing and `associate` asking for it.
    fn ready_to_reach_the_key_step(
        dir: &std::path::Path,
        interface: &str,
    ) -> (ControlSocket, UnixDatagram) {
        let peer = UnixDatagram::bind(dir.join(interface)).expect("bind the fake peer");
        let control = ControlSocket::open_in(dir, interface).expect("open the real control socket");
        peer.send_to(b"0", &control.ours).expect("queue the ADD_NETWORK reply");
        peer.send_to(b"OK", &control.ours).expect("queue the SET_NETWORK ssid reply");
        (control, peer)
    }

    /// A supplicant that stops existing mid-conversation is this end's send
    /// eventually failing, or its next read timing out instead — the two
    /// events race, and the kernel does not promise which one a caller sees.
    /// The refusal `associate` produces must not carry the key regardless of
    /// which. A responder thread, rather than a peer dropped up front,
    /// because dropping it before `associate` even starts would fail
    /// `ADD_NETWORK` instead — the first command, which never carries a key —
    /// and prove nothing about the one that does.
    #[test]
    fn a_gone_peer_does_not_leak_the_key_into_the_refusal() {
        let scratch = Scratch::new("gone");
        let peer = UnixDatagram::bind(scratch.0.join("wtest-gone")).expect("bind the fake peer");
        peer.set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .expect("set a bound on the responder thread's own wait");
        let mut control =
            ControlSocket::open_in(&scratch.0, "wtest-gone").expect("open the real control socket");
        let ours = control.ours.clone();

        // Answers exactly the two requests ahead of the key, then this
        // thread ends and the peer's socket closes with it — genuinely gone,
        // not a peer this test only pretends was never there.
        let responder = std::thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            for reply in [b"0".as_slice(), b"OK".as_slice()] {
                let _read = peer.recv(&mut buffer).expect("receive a request");
                peer.send_to(reply, &ours).expect("answer it");
            }
        });

        let refused =
            associate(&mut control, &ssid(), CANARY, 1, &mut || {}).expect_err("the peer is gone");
        responder.join().expect("the responder thread");

        assert!(matches!(refused, SupplicantError::Unreachable { .. }), "{refused:?}");
        assert!(!refused.to_string().contains(CANARY), "{refused}");
        assert!(!format!("{refused:?}").contains(CANARY), "{refused:?}");
    }

    /// A supplicant that never answers again is this end's read timing out —
    /// `REPLY_TIMEOUT_SECONDS` is 5, so this test takes about that long.
    #[test]
    fn a_silent_peer_does_not_leak_the_key_into_the_timeout() {
        let scratch = Scratch::new("silent");
        let (mut control, _peer) = ready_to_reach_the_key_step(&scratch.0, "wtest-silent");
        // `_peer` stays alive and simply never answers again.

        let refused = associate(&mut control, &ssid(), CANARY, 1, &mut || {})
            .expect_err("the peer never replies");
        assert!(matches!(refused, SupplicantError::Unreachable { .. }), "{refused:?}");
        assert!(!refused.to_string().contains(CANARY), "{refused}");
        assert!(!format!("{refused:?}").contains(CANARY), "{refused:?}");
    }

    /// A supplicant that answers `SET_NETWORK psk` with nothing but stale
    /// `ATTACH` events exhausts `EVENTS_SKIPPED`, and that refusal must not
    /// carry the key either.
    #[test]
    fn events_that_exhaust_the_skip_limit_do_not_leak_the_key() {
        let scratch = Scratch::new("events");
        let (mut control, peer) = ready_to_reach_the_key_step(&scratch.0, "wtest-events");
        for _ in 0..EVENTS_SKIPPED {
            peer.send_to(b"<3>CTRL-EVENT-SCAN-STARTED", &control.ours)
                .expect("queue an event");
        }

        let refused = associate(&mut control, &ssid(), CANARY, 1, &mut || {})
            .expect_err("only events ever come back");
        assert!(matches!(refused, SupplicantError::Unreachable { .. }), "{refused:?}");
        assert!(!refused.to_string().contains(CANARY), "{refused}");
        assert!(!format!("{refused:?}").contains(CANARY), "{refused:?}");
    }

    /// The flip side of the three refusals above: a request that succeeds
    /// still has to carry the key to the peer, unmodified.
    #[test]
    fn a_successful_request_carries_the_key_byte_for_byte() {
        let scratch = Scratch::new("byte-for-byte");
        let peer = UnixDatagram::bind(scratch.0.join("wtest-ok")).expect("bind the fake peer");
        let mut control =
            ControlSocket::open_in(&scratch.0, "wtest-ok").expect("open the real control socket");
        peer.send_to(b"OK", &control.ours).expect("queue the reply");

        let command = format!("SET_NETWORK 0 psk {CANARY}");
        let answer = control.request(&command).expect("the peer answered");
        assert_eq!(answer, "OK");

        let mut buffer = [0u8; 512];
        let read = peer.recv(&mut buffer).expect("the peer received something");
        assert_eq!(
            &buffer[..read],
            command.as_bytes(),
            "the key must reach the peer unmodified"
        );
    }
}
