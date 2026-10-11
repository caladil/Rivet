//! `serve`'s own suite.
//!
//! **Most of it runs with the `serve` feature OFF**, which is deliberate and is
//! the same argument the `screen` feature's split made: a test that can only be
//! compiled by a feature nothing enables is a test nothing runs. The module is
//! `#[cfg(any(feature = "serve", test))]` for exactly this reason, so
//! `cargo test --workspace` — which leaves every optional feature off — still
//! exercises every bound and every refusal below. Only [`super::tls`] needs
//! `rustls`, and `tools/test-all.sh` runs that half.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

/// A reader that hands over `at_a_time` bytes per call, so a request can be
/// made to arrive in pieces the way TCP really delivers one.
struct Dribble {
    bytes: Vec<u8>,
    at_a_time: usize,
}

impl std::io::Read for Dribble {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.bytes.len().min(buf.len()).min(self.at_a_time);
        buf[..n].copy_from_slice(&self.bytes[..n]);
        self.bytes.drain(..n);
        Ok(n)
    }
}

fn dribbled(text: &str, at_a_time: usize) -> Dribble {
    Dribble { bytes: text.as_bytes().to_vec(), at_a_time }
}

const CALL: &str = "POST /mcp/board HTTP/1.1\r\n\
                    Host: fleet\r\n\
                    Content-Type: application/json\r\n\
                    Content-Length: 40\r\n\
                    \r\n\
                    {\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}";

fn parse(text: &str) -> Result<Option<Head>, Refusal> {
    Head::parse(text.as_bytes(), &Bounds::default())
}

fn refusal(text: &str) -> Refusal {
    parse(text).expect_err("this request must be refused")
}

fn status(refusal: &Refusal) -> u16 {
    match refusal {
        Refusal::Answerable { status, .. } => *status,
        Refusal::Unanswerable(why) => panic!("expected an answerable refusal, got {why}"),
    }
}

// --- the defaults, and that they are the numbers the ADR argues -------------

/// The four defaults, asserted so that changing one is a deliberate act with a
/// test to update rather than a silent edit. `ADR-040` §4 is the argument for
/// each.
#[test]
fn the_defaults_are_the_ones_the_adr_argues_for() {
    let bounds = Bounds::default();
    assert_eq!(bounds.connections, 16);
    assert_eq!(bounds.head_bytes, 16 * 1024);
    assert_eq!(bounds.body_bytes, 1024 * 1024);
    assert_eq!(bounds.head_patience, Duration::from_secs(10));
}

/// **The whole point of the type**: a consumer with a reason states one number
/// and inherits the rest, rather than forking the logic to keep its own.
#[test]
fn one_number_can_be_stated_without_restating_the_others() {
    let proxyd = Bounds {
        head_patience: Duration::from_secs(15),
        head_bytes: 32 * 1024,
        ..Bounds::default()
    };
    assert_eq!(proxyd.head_patience, Duration::from_secs(15));
    assert_eq!(proxyd.head_bytes, 32 * 1024);
    assert_eq!(proxyd.connections, Bounds::default().connections);
    assert_eq!(proxyd.body_bytes, Bounds::default().body_bytes);
}

// --- framing -----------------------------------------------------------------

#[test]
fn an_incomplete_head_asks_for_more_rather_than_refusing() {
    assert_eq!(parse("").expect("more"), None);
    assert_eq!(parse("GET / HTTP/1.1\r\nHost: x\r\n").expect("more"), None);
}

#[test]
fn a_complete_head_yields_its_request_line_and_fields() {
    let head = parse(CALL).expect("parsed").expect("complete");
    assert_eq!(head.method, "POST");
    assert_eq!(head.target, "/mcp/board");
    assert_eq!(head.version, "HTTP/1.1");
    assert_eq!(head.content_length(), Some(40));
    assert_eq!(head.field("host"), Some("fleet"));
    assert!(!head.is_chunked());
}

/// Field names are case-insensitive (RFC 9110 §5.1) and browsers disagree about
/// the case they send, so a check that folded case in one place and not another
/// would work in one browser.
#[test]
fn field_names_are_matched_without_regard_to_case() {
    let head = parse("GET / HTTP/1.1\r\nCONTENT-TYPE: Application/JSON\r\n\r\n")
        .expect("parsed")
        .expect("complete");
    assert_eq!(head.field("content-type"), Some("Application/JSON"));
    assert_eq!(head.field("Content-Type"), Some("Application/JSON"));
}

#[test]
fn the_method_is_upper_cased_and_the_target_is_left_alone() {
    let head = parse("get /Some/Path?q=A%20B HTTP/1.1\r\n\r\n")
        .expect("parsed")
        .expect("complete");
    assert_eq!(head.method, "GET");
    assert_eq!(head.target, "/Some/Path?q=A%20B");
}

#[test]
fn the_body_offset_points_past_the_blank_line() {
    let head = parse(CALL).expect("parsed").expect("complete");
    assert_eq!(&CALL[head.body_at..], "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}");
}

// --- the bounds, each shown refusing ----------------------------------------

/// A head that never ends is refused rather than buffered for ever, and the
/// status says which bound it hit.
#[test]
fn a_head_with_no_blank_line_is_refused_at_the_bound() {
    let bounds = Bounds::default();
    let endless = format!("GET / HTTP/1.1\r\nX: {}\r\n", "a".repeat(bounds.head_bytes + 1));
    let refused = Head::parse(endless.as_bytes(), &bounds).expect_err("refused");
    assert_eq!(status(&refused), 431);
    assert!(refused.why().contains("no blank line"), "{refused}");
}

/// **A head that arrives complete and oversized is refused too**, which is the
/// half both hand-written parsers in this tree miss: each checks the bound only
/// while it is still looking for the blank line, so a whole 64 KiB head
/// delivered in one read passes a 16 KiB limit. The bound is on the head, not
/// on how it arrived.
#[test]
fn a_complete_head_past_the_bound_is_refused_and_not_only_an_unterminated_one() {
    let bounds = Bounds::default();
    let head = format!("GET / HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(bounds.head_bytes));
    let refused = Head::parse(head.as_bytes(), &bounds).expect_err("refused");
    assert_eq!(status(&refused), 431);
    assert!(refused.why().contains("past the"), "{refused}");
}

/// And the bound is the caller's: a head this size is fine for somebody who
/// said so.
#[test]
fn a_larger_bound_admits_the_same_head() {
    let small = Bounds::default();
    let big = Bounds { head_bytes: 128 * 1024, ..Bounds::default() };
    let head = format!("GET / HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(32 * 1024));
    assert!(Head::parse(head.as_bytes(), &small).is_err(), "the small bound must refuse it");
    assert!(
        Head::parse(head.as_bytes(), &big).expect("parsed").is_some(),
        "the caller's own bound must admit it"
    );
}

/// A body larger than the limit is refused **from its declared length**, before
/// a byte of it is read.
#[test]
fn an_oversized_body_is_refused_from_its_declared_length() {
    let bounds = Bounds::default();
    let head = format!(
        "PUT /payload/x HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
        bounds.body_bytes + 1
    );
    let refused = Head::parse(head.as_bytes(), &bounds).expect_err("refused");
    assert_eq!(status(&refused), 413);
}

/// `rivet-mcpd` accepts 1.2 GB. The point of the parameter is that it can.
#[test]
fn a_payload_sized_bound_admits_a_payload() {
    let bulk = Bounds { body_bytes: 1_200_000_000, ..Bounds::default() };
    let head = "PUT /payload/x.rvt HTTP/1.1\r\nContent-Length: 683631164\r\n\r\n";
    let parsed = Head::parse(head.as_bytes(), &bulk).expect("parsed").expect("complete");
    assert_eq!(parsed.content_length(), Some(683_631_164));
}

#[test]
fn the_connection_limit_holds_and_a_finished_connection_gives_its_slot_back() {
    let limit = Limit::of(3);
    let mut held: Vec<Slot> = (0..3).map(|n| limit.claim().unwrap_or_else(|| panic!("slot {n}"))).collect();
    assert_eq!(limit.live(), 3);
    assert!(limit.claim().is_none(), "the cap did not hold");
    drop(held.pop().expect("a slot"));
    assert_eq!(limit.live(), 2);
    let one_more = limit.claim().expect("the released slot came back");
    assert!(limit.claim().is_none(), "releasing one let two in");
    drop(one_more);
    drop(held);
    assert_eq!(limit.live(), 0, "a slot leaked");
}

/// **A limit is per-server and not per-process**, which is what the two copies
/// of this in the tree got wrong: both used a `static`, so a second listener in
/// one program would have shared the first one's budget silently.
#[test]
fn two_limits_do_not_share_a_budget() {
    let one = Limit::of(1);
    let two = Limit::of(1);
    let _held = one.claim().expect("the first");
    assert!(one.claim().is_none());
    assert!(two.claim().is_some(), "a separate limit must have its own budget");
}

/// A clone is a handle on the same counter, so claiming from an accept loop and
/// holding in a worker is one budget.
#[test]
fn a_cloned_limit_is_the_same_budget() {
    let limit = Limit::of(1);
    let handle = limit.clone();
    let _held = limit.claim().expect("the first");
    assert!(handle.claim().is_none(), "a clone must see the same count");
}

// --- the refusals a malformed request earns ---------------------------------

#[test]
fn a_request_line_that_is_not_one_is_refused() {
    for line in ["nonsense\r\n\r\n", "GET\r\n\r\n", "GET /x\r\n\r\n", "GET / HTTP/1.1 extra\r\n\r\n"] {
        assert_eq!(status(&refusal(line)), 400, "{line:?} must be refused");
    }
}

#[test]
fn a_version_this_server_does_not_speak_is_refused_as_such() {
    assert_eq!(status(&refusal("GET /x HTTP/0.9\r\n\r\n")), 505);
    assert_eq!(status(&refusal("GET /x HTTP/2\r\n\r\n")), 505);
}

#[test]
fn a_content_length_that_is_not_a_number_of_bytes_is_refused() {
    for value in ["", "abc", "+5", "5.0", "-1", " 5 5"] {
        let head = format!("POST /x HTTP/1.1\r\nContent-Length: {value}\r\n\r\n");
        assert_eq!(status(&refusal(&head)), 400, "Content-Length: {value:?}");
    }
}

#[test]
fn a_line_that_is_not_a_header_field_is_refused() {
    assert_eq!(status(&refusal("GET /x HTTP/1.1\r\nHost x\r\n\r\n")), 400);
}

/// **Two lengths that disagree is request smuggling, not a typo.** A server
/// that takes one and a proxy in front that takes the other disagree about
/// where this request ends and the next one begins — and `rivet-proxyd` is a
/// proxy in front of exactly these servers.
#[test]
fn two_disagreeing_content_lengths_are_refused() {
    let head = "POST /x HTTP/1.1\r\nContent-Length: 5\r\nContent-Length: 7\r\n\r\n";
    let refused = refusal(head);
    assert_eq!(status(&refused), 400);
    assert!(refused.why().contains("no one length"), "{refused}");
}

/// **A repeated `Content-Length` is refused even when both copies agree** —
/// `.rulings/` R-210 D17. A matching pair is no safer than a disagreeing one:
/// nothing downstream of this parser should ever have to decide which of two
/// same-named headers was the real one.
#[test]
fn two_identical_content_lengths_are_refused_too() {
    let head = "POST /x HTTP/1.1\r\nContent-Length: 5\r\nContent-Length: 5\r\n\r\n";
    let refused = refusal(head);
    assert_eq!(status(&refused), 400);
    assert!(refused.why().contains("no one length"), "{refused}");
}

/// Both framings at once is the classic smuggling shape, and RFC 9112 §6.1 says
/// to refuse it.
#[test]
fn a_request_carrying_both_framings_is_refused() {
    let head = "POST /x HTTP/1.1\r\nTransfer-Encoding: chunked\r\nContent-Length: 5\r\n\r\n";
    let refused = refusal(head);
    assert_eq!(status(&refused), 400);
    assert!(refused.why().contains("two framings"), "{refused}");
}

/// Chunked alone is **recorded so it can be refused**, not decoded. The caller
/// decides; this only tells it.
#[test]
fn chunked_alone_is_reported_and_not_refused_here() {
    let head = parse("POST /x HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n")
        .expect("parsed")
        .expect("complete");
    assert!(head.is_chunked());
    assert_eq!(head.content_length(), None);
}

// --- what a refusal looks like on the wire ----------------------------------

#[test]
fn an_answerable_refusal_renders_a_status_line_and_a_length() {
    let refused = refusal("GET /x HTTP/0.9\r\n\r\n");
    let wire = refused.response().expect("there is somebody to answer");
    assert!(wire.starts_with("HTTP/1.1 505 HTTP Version Not Supported\r\n"), "{wire}");
    assert!(wire.contains("Connection: close\r\n"), "{wire}");
    let (head, body) = wire.split_once("\r\n\r\n").expect("a blank line");
    let stated: usize = head
        .lines()
        .find_map(|l| l.strip_prefix("Content-Length: "))
        .expect("a length")
        .trim()
        .parse()
        .expect("a number");
    assert_eq!(stated, body.len(), "the stated length must be the body's");
}

/// **A peer that vanished has nobody to answer**, and the type says so rather
/// than handing back a response to write into a closed socket.
#[test]
fn an_unanswerable_refusal_renders_nothing() {
    let mut empty = dribbled("", 16);
    let refused = read_head(&mut empty, &Bounds::default()).expect_err("refused");
    assert!(matches!(refused, Refusal::Unanswerable(_)), "{refused}");
    assert!(refused.response().is_none());
    assert!(refused.why().contains("closed before"), "{refused}");
}

// --- reading, which is where the defect was ---------------------------------

/// **A request delivered one byte at a time is still read whole.** The single
/// `read(&mut [0u8; 1024])` this replaces took whatever one call returned.
#[test]
fn a_request_delivered_in_pieces_is_read_whole() {
    for at_a_time in [1usize, 7, 64, 4096] {
        let mut reader = dribbled(CALL, at_a_time);
        let (head, rest) = read_head(&mut reader, &Bounds::default())
            .unwrap_or_else(|e| panic!("chunk {at_a_time}: {e}"));
        assert_eq!(head.target, "/mcp/board", "chunk {at_a_time}");
        let body = read_body(&mut reader, &head, &rest)
            .unwrap_or_else(|e| panic!("chunk {at_a_time}: {e}"));
        assert_eq!(
            String::from_utf8_lossy(&body),
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}",
            "chunk {at_a_time}"
        );
    }
}

/// Only `Content-Length` bytes are taken, so a pipelined second request cannot
/// be glued onto the first one's body.
#[test]
fn the_body_stops_at_content_length() {
    let two = format!("{CALL}POST /mcp/other HTTP/1.1\r\n\r\n");
    let mut reader = dribbled(&two, 4096);
    let (head, rest) = read_head(&mut reader, &Bounds::default()).expect("read");
    let body = read_body(&mut reader, &head, &rest).expect("read");
    assert_eq!(body.len(), 40);
    assert!(
        !String::from_utf8_lossy(&body).contains("other"),
        "a second request leaked into the first's body"
    );
}

/// A peer that promises a body and then hangs up is an error, not a shorter
/// request that gets forwarded to a machine.
#[test]
fn a_truncated_body_is_an_error_and_not_a_shorter_request() {
    let cut = &CALL[..CALL.len() - 10];
    let mut reader = dribbled(cut, 4096);
    let (head, rest) = read_head(&mut reader, &Bounds::default()).expect("read");
    let refused = read_body(&mut reader, &head, &rest).expect_err("refused");
    assert!(refused.why().contains("promised"), "{refused}");
}

/// A request with no body reads as a body of nothing, not as a wait.
#[test]
fn a_request_with_no_content_length_has_no_body() {
    let mut reader = dribbled("GET /x HTTP/1.1\r\nHost: h\r\n\r\n", 4096);
    let (head, rest) = read_head(&mut reader, &Bounds::default()).expect("read");
    assert_eq!(head.content_length(), None);
    assert!(read_body(&mut reader, &head, &rest).expect("read").is_empty());
}

/// **`read_body_into` holds nothing**, which is what lets one function serve a
/// JSON call and a 683 MB payload. Here the sink is a hasher's stand-in: a
/// counter that never keeps a byte.
#[test]
fn a_body_can_be_streamed_without_being_held() {
    struct Counting(u64);
    impl std::io::Write for Counting {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 += buf.len() as u64;
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let big = 5 * 1024 * 1024u64;
    let text = format!("PUT /payload/x HTTP/1.1\r\nContent-Length: {big}\r\n\r\n");
    let mut bytes = text.into_bytes();
    bytes.extend(std::iter::repeat_n(b'z', usize::try_from(big).expect("fits")));
    let mut reader = std::io::Cursor::new(bytes);

    let bounds = Bounds { body_bytes: 1_200_000_000, ..Bounds::default() };
    let (head, rest) = read_head(&mut reader, &bounds).expect("read");
    let mut sink = Counting(0);
    let written = read_body_into(&mut reader, &head, &rest, &mut sink).expect("streamed");
    assert_eq!(written, big);
    assert_eq!(sink.0, big);
}

// --- one absolute deadline through a fragmented head -------------------------

/// A fake socket for testing an absolute head deadline without a real one.
///
/// **A queued fragment arrives after `gap`** — standing in for a peer that
/// sends one small piece and then pauses, comfortably inside whatever
/// per-read timeout happens to be configured. **Once the queue is empty,
/// `read` waits out whatever `set_read_deadline` most recently configured and
/// then reports a timeout** — standing in for a peer that has gone silent for
/// good, the way a real socket's own read timeout would return control
/// rather than block forever.
///
/// This is what makes [`read_head_by_deadline`]'s own accounting the thing
/// under test rather than the OS's: nothing here opens a socket, and nothing
/// here sleeps longer than the timeout the function under test itself asked
/// for.
struct Socket {
    fragments: std::collections::VecDeque<&'static str>,
    gap: Duration,
    timeout: std::rc::Rc<std::cell::Cell<Duration>>,
}

impl std::io::Read for Socket {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if let Some(chunk) = self.fragments.pop_front() {
            std::thread::sleep(self.gap);
            let bytes = chunk.as_bytes();
            let n = bytes.len().min(buf.len());
            buf[..n].copy_from_slice(&bytes[..n]);
            return Ok(n);
        }
        std::thread::sleep(self.timeout.get());
        Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "no data within the read timeout",
        ))
    }
}

/// A `Socket` and the `set_read_deadline` closure that shares its timeout,
/// built together because the closure is what a real caller passes and the
/// socket is what stands in for the one it would set the timeout on.
fn socket(
    fragments: &[&'static str],
    gap: Duration,
) -> (Socket, impl FnMut(Duration) -> std::io::Result<()>) {
    let timeout = std::rc::Rc::new(std::cell::Cell::new(Duration::ZERO));
    let reader = Socket { fragments: fragments.iter().copied().collect(), gap, timeout: std::rc::Rc::clone(&timeout) };
    let set_read_deadline = move |d| {
        timeout.set(d);
        Ok(())
    };
    (reader, set_read_deadline)
}

const SLOW_HEAD: [&str; 5] = ["GET /x HTTP/1.1\r\n", "Host: h\r\n", "A: 1\r\n", "B: 2\r\n", "\r\n"];

/// **Fragments that each arrive comfortably inside a per-read timeout can
/// still add up to more than the promised total.** `read_head` bounds each
/// read and not the call; this carries one `deadline` through every read
/// instead, and must refuse once the SUM passes it, not once any single read
/// does. Five 80 ms fragments (400 ms) against a 250 ms deadline: each gap is
/// a fraction of what a real per-read timeout would allow, and the refusal
/// still lands, near the deadline and before the fifth fragment — the one
/// carrying the terminating blank line — is ever read.
#[test]
fn fragments_each_inside_a_per_read_timeout_can_still_exceed_the_deadline() {
    let (mut reader, set_read_deadline) = socket(&SLOW_HEAD, Duration::from_millis(80));
    let deadline = std::time::Instant::now() + Duration::from_millis(250);

    let refusal = read_head_by_deadline(&mut reader, &Bounds::default(), deadline, set_read_deadline)
        .expect_err("five 80ms fragments must not fit inside a 250ms deadline");
    assert!(refusal.why().contains("deadline"), "{refusal}");
}

/// **A complete head that arrives with room to spare is still accepted** —
/// the fix's other half. Carrying one deadline through the loop must not
/// refuse a request that was always going to finish in time.
#[test]
fn a_complete_head_arriving_inside_the_deadline_is_accepted() {
    let (mut reader, set_read_deadline) = socket(&SLOW_HEAD, Duration::from_millis(80));
    let deadline = std::time::Instant::now() + Duration::from_millis(500);

    let (head, _) = read_head_by_deadline(&mut reader, &Bounds::default(), deadline, set_read_deadline)
        .expect("five 80ms fragments comfortably fit inside a 500ms deadline");
    assert_eq!(head.target, "/x");
}

/// **A peer that never sends anything is refused rather than held forever.**
/// No fragments are queued, so the first read waits out the timeout this
/// function itself configures — the full deadline, on the first read — and
/// then reports nothing arrived, exactly what a real socket's own read
/// timeout would do.
#[test]
fn a_silent_peer_is_refused_rather_than_held_forever() {
    let (mut reader, set_read_deadline) = socket(&[], Duration::from_millis(80));
    let deadline = std::time::Instant::now() + Duration::from_millis(120);

    let refusal = read_head_by_deadline(&mut reader, &Bounds::default(), deadline, set_read_deadline)
        .expect_err("a silent peer must be refused");
    assert!(refusal.why().contains("timeout"), "{refusal}");
}

#[cfg(feature = "serve")]
mod tls {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::super::Tls;

    /// A directory of its own, removed when the test ends.
    ///
    /// **The suite mints its own material into a temporary path** — `AGENTS.md`
    /// §8.3 — so nothing here is committed and nothing reads a real key.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(what: &str) -> Self {
            static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("rivet-sdk-serve-{}-{what}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).expect("a scratch directory");
            Self(path)
        }

        fn write(&self, name: &str, text: &str) {
            std::fs::write(self.0.join(name), text).expect("write");
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A self-signed leaf, as PEM.
    fn leaf(name: &str) -> (String, String) {
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).expect("a pair");
        let mut params = rcgen::CertificateParams::new(vec![name.to_owned()]).expect("params");
        params.is_ca = rcgen::IsCa::NoCa;
        let cert = params.self_signed(&key).expect("a leaf");
        (cert.pem(), key.serialize_pem())
    }

    /// An authority, as PEM.
    fn authority() -> String {
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).expect("a pair");
        let mut params = rcgen::CertificateParams::new(vec!["rivet-test-ca".to_owned()])
            .expect("params");
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.self_signed(&key).expect("a root").pem()
    }

    #[test]
    fn a_certificate_and_its_key_build_a_server() {
        let scratch = Scratch::new("pair");
        let (cert, key) = leaf("rivet-01.local");
        scratch.write("tls-cert", &cert);
        scratch.write("tls-key", &key);

        Tls::in_directory(&scratch.0, "tls-cert", "tls-key")
            .server_config()
            .expect("a server configuration");
    }

    /// **The refusal that matters most**, because it is silent otherwise: a
    /// certificate and a key from different pairs.
    #[test]
    fn a_certificate_and_a_key_that_do_not_go_together_are_refused() {
        let scratch = Scratch::new("mismatch");
        let (cert, _) = leaf("rivet-01.local");
        let (_, other_key) = leaf("rivet-02.local");
        scratch.write("tls-cert", &cert);
        scratch.write("tls-key", &other_key);

        let why = Tls::in_directory(&scratch.0, "tls-cert", "tls-key")
            .server_config()
            .expect_err("refused");
        assert!(why.contains("do not go together"), "{why}");
    }

    /// A missing file names itself and says how to install it. The machine that
    /// hits this is headless and the message is all its operator gets.
    #[test]
    fn a_missing_file_names_itself_and_the_fix() {
        let scratch = Scratch::new("missing");
        let why = Tls::in_directory(&scratch.0, "tls-cert", "tls-key")
            .server_config()
            .expect_err("refused");
        assert!(why.contains("tls-cert"), "{why}");
        assert!(why.contains("install-secret"), "{why}");
    }

    /// A PEM file with no certificate in it is refused rather than producing a
    /// server that presents nothing.
    #[test]
    fn a_certificate_file_with_no_certificate_is_refused() {
        let scratch = Scratch::new("empty-cert");
        let (_, key) = leaf("rivet-01.local");
        scratch.write("tls-cert", "# nothing here\n");
        scratch.write("tls-key", &key);

        let why = Tls::in_directory(&scratch.0, "tls-cert", "tls-key")
            .server_config()
            .expect_err("refused");
        assert!(why.contains("carries no certificate"), "{why}");
    }

    #[test]
    fn a_caller_certificate_can_be_required_against_an_authority() {
        let scratch = Scratch::new("client-auth");
        let (cert, key) = leaf("rivet-01.local");
        scratch.write("tls-cert", &cert);
        scratch.write("tls-key", &key);
        scratch.write("tls-root", &authority());

        Tls::in_directory(&scratch.0, "tls-cert", "tls-key")
            .requiring_a_caller_certificate_from("tls-root")
            .server_config()
            .expect("a server that requires a caller certificate");
    }

    /// **An empty authority would refuse every caller**, so it is refused at
    /// startup instead of at the first connection, where the only party told
    /// about it is the one that must not be.
    #[test]
    fn an_authority_with_no_certificate_in_it_is_refused_at_startup() {
        let scratch = Scratch::new("empty-root");
        let (cert, key) = leaf("rivet-01.local");
        scratch.write("tls-cert", &cert);
        scratch.write("tls-key", &key);
        scratch.write("tls-root", "# an operator emptied this\n");

        let why = Tls::in_directory(&scratch.0, "tls-cert", "tls-key")
            .requiring_a_caller_certificate_from("tls-root")
            .server_config()
            .expect_err("refused");
        assert!(why.contains("would refuse every caller"), "{why}");
    }

    /// **One provider in the tree, and it is `rivet-tls`'s.** `rivet-proxyd`
    /// wrote its own returning `ring::default_provider()` with a comment saying
    /// it matched the others; this is that comment as a test.
    #[test]
    fn the_provider_is_the_one_rivet_tls_names() {
        let ours = super::super::provider();
        let theirs = rivetos_tls::provider();
        assert_eq!(ours.cipher_suites, theirs.cipher_suites);
        let named = |p: &rustls::crypto::CryptoProvider| {
            p.kx_groups.iter().map(|g| g.name()).collect::<Vec<_>>()
        };
        assert_eq!(named(&ours), named(&theirs));
    }
}
