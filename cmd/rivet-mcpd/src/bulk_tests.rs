//! What the bulk endpoint refuses, and what it keeps.
//!
//! The ones that matter: a digest that does not match leaves **nothing**, and a
//! transfer that dies part-way leaves nothing either — because a partial
//! payload under the final name is one `rivet apply install` would pick up.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

fn head(text: &str) -> Result<Option<(Request, usize)>, Response> {
    parse_head(text.as_bytes())
}

fn digest_of(bytes: &[u8]) -> String {
    let mut h = rivet_crypto::Blake3::new();
    h.start();
    h.write(bytes);
    hex(&h.finish())
}

/// A well-formed request is read, and the body offset points past the blank
/// line — the first read almost always carries some of the body.
#[test]
fn a_put_is_parsed_and_the_body_offset_is_past_the_head() {
    let text = "PUT /payload/thing.rvt HTTP/1.1\r\nRivet-Blake3: ab\r\nContent-Length: 9\r\n\r\nbody";
    let (request, at) = head(text).expect("well formed").expect("complete");
    assert_eq!(request.method, "PUT");
    assert_eq!(request.target, "/payload/thing.rvt");
    assert_eq!(request.digest.as_deref(), Some("ab"));
    assert_eq!(request.length, Some(9));
    assert_eq!(&text[at..], "body");
}

/// An incomplete head asks for more rather than guessing.
#[test]
fn a_partial_head_is_not_an_error() {
    assert_eq!(head("PUT /payload/x HTTP/1.1\r\nRivet-Bl"), Ok(None));
}

/// Header names are case-insensitive on the wire and this reads them that way.
#[test]
fn headers_are_matched_without_regard_to_case() {
    let text = "PUT /payload/x HTTP/1.1\r\nRIVET-BLAKE3: AB12\r\ncontent-length: 4\r\n\r\n";
    let (request, _) = head(text).expect("well formed").expect("complete");
    assert_eq!(request.digest.as_deref(), Some("ab12"));
    assert_eq!(request.length, Some(4));
}

fn a_request(target: &str, digest: Option<&str>, length: Option<u64>) -> Request {
    Request {
        method: "PUT".to_string(),
        target: target.to_string(),
        digest: digest.map(str::to_string),
        length,
        content_type: None,
        content_encoding: None,
        chunked: false,
    }
}

/// **`Content-Encoding` is read**, for `crate::git` to hand on: a gzipped
/// fetch body the CGI is not told about reaches upload-pack as garbage.
#[test]
fn a_content_encoding_is_recognised() {
    let text = "POST /r.git/git-upload-pack HTTP/1.1\r\nContent-Encoding: gzip\r\nContent-Length: 9\r\n\r\n";
    let (request, _) = head(text).expect("well formed").expect("complete");
    assert_eq!(request.content_encoding.as_deref(), Some("gzip"));
    let plain = "POST /r.git/git-upload-pack HTTP/1.1\r\nContent-Length: 9\r\n\r\n";
    assert_eq!(head(plain).expect("well formed").expect("complete").0.content_encoding, None);
}

/// **A payload with no digest is refused, not defaulted.** The digest is about
/// the transfer, which is the part a signature does not cover.
#[test]
fn a_put_with_no_digest_is_refused_and_says_why() {
    let refusal = accept(&a_request("/payload/x.rvt", None, Some(4))).expect_err("refused");
    assert_eq!(refusal.status, 400);
    assert!(refusal.body.contains("nobody checked"), "{}", refusal.body);
}

/// No `Content-Length` is refused before anything is written, because a write
/// with no stated size is one nothing can refuse before it fills the card.
#[test]
fn a_put_with_no_length_is_refused_with_411() {
    let d = "a".repeat(64);
    let refusal = accept(&a_request("/payload/x.rvt", Some(&d), None)).expect_err("refused");
    assert_eq!(refusal.status, 411);
    assert!(refusal.body.contains("fills the filesystem"), "{}", refusal.body);
}

/// **No path escapes.** A closed rule, so anything that is not the shape
/// `cargo rivet package` produces is refused whether or not it is dangerous.
#[test]
fn a_name_that_could_escape_the_directory_is_refused() {
    let d = "a".repeat(64);
    for target in [
        "/payload/../../etc/passwd",
        "/payload/sub/dir.rvt",
        "/payload/.hidden",
        "/payload/",
        "/payload/..",
    ] {
        let refusal =
            accept(&a_request(target, Some(&d), Some(4))).expect_err("must refuse {target}");
        assert!(
            refusal.status == 400 || refusal.status == 404,
            "{target} gave {}",
            refusal.status
        );
    }
    // ...and the real shape is accepted.
    let ok = accept(&a_request(
        "/payload/rivet-0c17f74-aarch64-rpi4-update.rvt",
        Some(&d),
        Some(4),
    ))
    .expect("a name cargo rivet package produces");
    assert_eq!(ok.0, "rivet-0c17f74-aarch64-rpi4-update.rvt");
}

/// **A PUT may not write a file a MECHANISM owns.** The payload directory was `/state`
/// itself until 2026-09-12, so every one of them was a plain name away; it is
/// the artifact store now, and the rule is kept for the reasons `safe_name`
/// gives.
///
/// Audit of 2026-09-03, register R-31. `deploy-request` is letters and a
/// hyphen: it needs no separator, no leading dot and no traversal, so every
/// clause of the rule above passed it. Writing it forges the principal
/// `ADR-028` §4.5 records and skips `rivet apply install` — the thing
/// `rivet apply edit` refuses this exact path by name to prevent.
///
/// Asserted as a list of what must be REFUSED, but the mechanism is the
/// closed rule: a name must end in `rivet_core::package::kind::SUFFIX`. These
/// are the cases that motivated it, not the extent of it — a `/state` file
/// invented next year is refused by the same clause without this list being
/// touched.
#[test]
fn a_put_cannot_write_a_file_a_mechanism_owns() {
    let d = "a".repeat(64);
    for target in [
        // The install channel, and its half-written temporary.
        "/payload/deploy-request",
        "/payload/deploy-request.new",
        // `rivet-store`'s clock, named in `reserved_paths`' own notes as the
        // gap it could not close.
        "/payload/clock",
        // The journal and the application tree, both `tree: true` reservations.
        "/payload/log",
        "/payload/app",
        // A suffix has to be a suffix of something.
        "/payload/.rvt",
        // Close but not the artifact: `ADR-027` §7 fixes one shape.
        "/payload/payload.rvt.part",
        "/payload/update.tar",
    ] {
        let refusal = accept(&a_request(target, Some(&d), Some(4)))
            .expect_err("a mechanism's file must not be writable over the bulk channel");
        assert!(
            refusal.status == 400 || refusal.status == 404,
            "{target} gave {}",
            refusal.status
        );
    }
    // And an application payload, the other legitimate shape, still lands.
    let ok = accept(&a_request(
        "/payload/hello-0.1.0-aarch64-application.rvt",
        Some(&d),
        Some(4),
    ))
    .expect("an application payload is exactly what this endpoint is for");
    assert_eq!(ok.0, "hello-0.1.0-aarch64-application.rvt");
}

/// A method this endpoint does not serve says which ones exist, and where git
/// actually is — a GET reaching here missed git's routes, which are served
/// on the same listener (`crate::git`). Until 2026-09-14 this said git was
/// "phase 3", two weeks after it had been built.
#[test]
fn a_get_is_refused_by_name_and_names_where_git_is_served() {
    let mut request = a_request("/payload/x.rvt", Some(&"a".repeat(64)), Some(4));
    request.method = "GET".to_string();
    let refusal = accept(&request).expect_err("refused");
    assert_eq!(refusal.status, 405);
    assert!(!refusal.body.contains("phase 3"), "{}", refusal.body);
    assert!(refusal.body.contains("IS served"), "{}", refusal.body);
    assert!(refusal.body.contains("/info/refs"), "{}", refusal.body);
}

/// An oversized body is refused before a byte of it is read.
#[test]
fn a_body_larger_than_the_ceiling_is_refused_before_it_is_read() {
    let d = "a".repeat(64);
    let refusal = accept(&a_request("/payload/x.rvt", Some(&d), Some(MAX_BYTES + 1)))
        .expect_err("refused");
    assert_eq!(refusal.status, 413);
}

/// A store of its own, in a temporary directory, removed when the guard drops.
///
/// **This is what changed on 2026-09-12 and it is the point of the change.**
/// `receive` used to write to `/state` with no way to redirect it, so every
/// assertion about what it leaves on disk was skipped on every machine a
/// developer or CI has — the tests below ran `return` and passed. Threading a
/// `Store` in (see `rivet_core::artifact::Store`) makes the whole function
/// reachable here, on Windows and in the container alike, and makes a prune
/// that deletes things safe to test: the only artifacts it can reach are the
/// ones this test put there.
struct Scratch {
    store: artifact::Store,
    dir: std::path::PathBuf,
    door: Box<dyn Door>,
}

/// **A door that admits everything and remembers being asked.**
///
/// For the tests below that are about the TRANSFER — streaming, the digest,
/// the drain, retention — where the bodies are a few bytes of text and a signed
/// artifact would be noise. What it proves is the order around the decision:
/// which transfers reach the door at all. The door's own decision is tested
/// with real signed artifacts and [`TrustStoreDoor`], further down.
#[derive(Default)]
struct Recording {
    refuse: bool,
    asked: std::cell::RefCell<Vec<String>>,
}

impl Door for Recording {
    fn admit(&self, path: &str, _length: u64) -> Result<String, Refusal> {
        self.asked.borrow_mut().push(path.to_string());
        if self.refuse {
            Err(Refusal::Artifact("a test door that refuses everything".to_string()))
        } else {
            Ok("\x20 verified by a test door that admits everything\n".to_string())
        }
    }
}

impl Scratch {
    fn new(label: &str) -> Self {
        Self::with_door(label, Box::new(Recording::default()))
    }

    fn with_door(label: &str, door: Box<dyn Door>) -> Self {
        // The thread name and the process id, because `cargo test` runs these
        // concurrently in one process and two of them sharing a store would
        // race over the same prune.
        let dir = std::env::temp_dir().join(format!(
            "rivet-bulk-{}-{label}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        Self {
            store: artifact::Store::at(dir.join("artifacts").to_string_lossy().into_owned()),
            dir,
            door,
        }
    }

    /// Send `body` in one piece, as the `PUT` handler would after a head parse
    /// that happened to read the whole request.
    fn put(&self, name: &str, body: &[u8]) -> Result<Response, Response> {
        let expected = hex32(&digest_of(body)).expect("64 hex characters");
        let mut empty = |_: &mut [u8]| Ok(0);
        receive(
            &self.store,
            &*self.door,
            name,
            expected,
            body.len() as u64,
            body,
            &mut empty,
        )
    }

    /// Send `body` the way a socket does: `first` bytes arrived with the head
    /// and the rest is still coming, handed over by the `more` closure.
    ///
    /// Answers the response **and how much of the body was actually read**,
    /// which is the whole of what the drain has to be asserted on.
    fn put_in_two_parts(
        &self,
        name: &str,
        body: &[u8],
        first: usize,
    ) -> (Result<Response, Response>, usize) {
        let expected = hex32(&digest_of(body)).expect("64 hex characters");
        let rest = &body[first..];
        let mut at = 0usize;
        let response = {
            let mut feed = |into: &mut [u8]| -> std::io::Result<usize> {
                let n = into.len().min(rest.len() - at);
                into[..n].copy_from_slice(&rest[at..at + n]);
                at += n;
                Ok(n)
            };
            receive(
                &self.store,
                &*self.door,
                name,
                expected,
                body.len() as u64,
                &body[..first],
                &mut feed,
            )
        };
        (response, first + at)
    }

    /// Every object name in the store, sorted, so an assertion is stable.
    fn holds(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.store.objects_dir()) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .flatten()
            .filter_map(|object| {
                std::fs::read_dir(object.path())
                    .ok()?
                    .flatten()
                    .next()
                    .map(|f| f.file_name().to_string_lossy().into_owned())
            })
            .collect();
        names.sort();
        names
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// An `ADR-027` name for a product and version, so retention has something to
/// group by.
fn app(product: &str, version: &str) -> String {
    format!("{product}-{version}-aarch64-application.rvt")
}

/// **The transfer lands in the store, under its own digest, keeping its name.**
/// `ADR-052` §7 end to end through this function.
#[test]
fn a_transferred_artifact_lands_in_the_store_under_its_digest() {
    let scratch = Scratch::new("lands");
    let body = b"an artifact's bytes";
    let name = app("photo-index", "0.1.0");

    let response = scratch.put(&name, body).expect("a good transfer");
    assert_eq!(response.status, 201);

    let digest = artifact::Digest::new(&digest_of(body)).expect("a digest");
    let path = scratch.store.object_path(&digest, &name);
    assert!(std::path::Path::new(&path).exists(), "{path} is not there");
    assert_eq!(std::fs::read(&path).expect("the bytes"), body);

    // The response names the digest, because that is what an install takes.
    assert!(response.body.contains(digest.as_str()), "{}", response.body);
    assert!(
        response.body.contains(&format!("rivet apply install {digest} confirm")),
        "{}",
        response.body
    );
    // And nothing is left in the incoming directory.
    assert!(
        !std::path::Path::new(&scratch.store.incoming_for(&digest)).exists(),
        "a finished transfer left its incoming directory behind"
    );
}

/// **The digest is checked and a mismatch keeps nothing** — neither the file nor
/// the directory it was being built in.
#[test]
fn a_body_whose_digest_is_wrong_is_refused_and_leaves_nothing() {
    let door = Recording::default();
    let scratch = Scratch::new("mismatch");
    let body = b"the bytes that were sent";
    let wrong = [0u8; 32];
    let mut empty = |_: &mut [u8]| Ok(0);
    let refusal = receive(
        &scratch.store,
        &door,
        "t.rvt",
        wrong,
        body.len() as u64,
        body,
        &mut empty,
    )
    .expect_err("a wrong digest must be refused");

    assert_eq!(refusal.status, 409);
    assert!(refusal.body.contains("NOTHING WAS KEPT"), "{}", refusal.body);
    assert!(scratch.holds().is_empty(), "{:?}", scratch.holds());
    let claimed = artifact::Digest::new(&hex(&wrong)).expect("a digest");
    assert!(
        !std::path::Path::new(&scratch.store.incoming_for(&claimed)).exists(),
        "a refused transfer left its incoming directory behind"
    );
    // **And the door was never asked.** A corrupted transfer is the wire's
    // fault, and it is reported as one before any signature work is done.
    assert!(door.asked.borrow().is_empty(), "{:?}", door.asked.borrow());
}

/// **A connection that dies mid-body keeps nothing**, and in particular leaves
/// nothing under `objects/` — the directory everything that resolves a digest
/// reads.
///
/// **And it fails the TRANSFER before any signature work is done**: the door
/// is not asked about a body that never finished arriving. A truncated artifact
/// reported as "signed by nobody this machine trusts" would send an operator to
/// look at a key that is fine.
#[test]
fn a_transfer_that_stops_early_leaves_nothing_and_never_reaches_the_door() {
    let door = Recording::default();
    let scratch = Scratch::new("truncated");
    let body = b"the first half";
    let expected = hex32(&digest_of(b"the first half and the second")).expect("hex");
    let mut nothing_more = |_: &mut [u8]| Ok(0);
    let refusal = receive(
        &scratch.store,
        &door,
        &app("photo-index", "0.1.0"),
        expected,
        1_000,
        body,
        &mut nothing_more,
    )
    .expect_err("a truncated body must be refused");

    assert_eq!(refusal.status, 400);
    assert!(scratch.holds().is_empty(), "{:?}", scratch.holds());
    assert!(door.asked.borrow().is_empty(), "{:?}", door.asked.borrow());
}

/// **Sending the same artifact twice succeeds twice**, because a retried deploy
/// is ordinary and the bytes cannot have changed — the name is their hash.
#[test]
fn re_sending_an_artifact_this_machine_already_holds_succeeds() {
    let scratch = Scratch::new("idempotent");
    let body = b"an artifact sent twice";
    let name = app("photo-index", "0.1.0");

    let first = scratch.put(&name, body).expect("a good transfer");
    assert_eq!(first.status, 201);
    let second = scratch.put(&name, body).expect("a re-send");
    assert_eq!(second.status, 201);
    assert!(second.body.contains("already holds"), "{}", second.body);
    assert_eq!(scratch.holds(), vec![name]);
}

/// **A machine that has already answered still reads the body out**, and this
/// is the defect of 2026-09-12 stated as a test.
///
/// On `rivet-01` the client reported *"the transfer failed after 196608 of
/// 402198 bytes: An established connection was aborted"* — the identical offset
/// on two runs — while this endpoint logged `201 Created` and its store held the
/// artifact complete. Three 64 KiB writes had landed before the reset got back.
/// The transfer had worked and the operator was shown a failure, so the install
/// that should have followed was never asked for.
///
/// The short-circuit answered without draining, and the close that followed —
/// with bytes still arriving — is a TCP `RST` rather than a `FIN`. **What is
/// asserted here is the half of that a unit test can reach**: that the
/// endpoint consumes the whole declared body before it answers. Whether the
/// peer then gets its answer is a question about sockets and needs the board.
///
/// It is asserted in BYTES READ rather than in the response text, because the
/// text is a report and the read is the behaviour.
#[test]
fn a_re_send_of_an_artifact_already_held_reads_the_body_out_before_answering() {
    let scratch = Scratch::new("drained");
    // Larger than one 64 KiB buffer several times over, so the drain has to
    // loop — 196608 is exactly three of them, which is what the board reported.
    let body: Vec<u8> = (0..200_000u32).map(|i| u8::try_from(i % 251).unwrap_or(0)).collect();
    let name = app("photo-index", "0.1.0");

    let first = scratch.put(&name, &body).expect("a good transfer");
    assert_eq!(first.status, 201);

    // The re-send: 64 KiB arrived with the head, the rest is still on the wire.
    let (second, consumed) = scratch.put_in_two_parts(&name, &body, 64 * 1024);
    let second = second.expect("a re-send of a held artifact succeeds");
    assert_eq!(second.status, 201);
    assert!(second.body.contains("already holds"), "{}", second.body);
    assert_eq!(
        consumed,
        body.len(),
        "the endpoint answered and left {} of {} bytes unread — closing on those is a RESET, \
         and a reset destroys the answer on its way back",
        body.len() - consumed,
        body.len(),
    );
    // And it says so, because an operator watching a re-send spend the wire time
    // deserves to read why.
    assert!(second.body.contains("drained"), "{}", second.body);

    // Nothing was written twice: the store still holds exactly one object, and
    // the drain did not leave an incoming directory behind.
    assert_eq!(scratch.holds(), vec![name]);
    let digest = artifact::Digest::new(&digest_of(&body)).expect("a digest");
    assert!(!std::path::Path::new(&scratch.store.incoming_for(&digest)).exists());
}

/// **A peer that vanishes mid-drain is still answered**, because the artifact is
/// held and that answer was true before the drain started. A drain that could
/// fail the transfer would have turned a success into an error for the second
/// time in one function.
#[test]
fn a_drain_that_cannot_finish_does_not_fail_a_transfer_that_already_succeeded() {
    let scratch = Scratch::new("drain-cut");
    let body = b"an artifact this machine already has";
    let name = app("photo-index", "0.1.0");
    scratch.put(&name, body).expect("a good transfer");

    let expected = hex32(&digest_of(body)).expect("64 hex characters");
    // The head carried four bytes and then the connection died.
    let mut gone = |_: &mut [u8]| Ok(0);
    let response = receive(
        &scratch.store,
        &*scratch.door,
        &name,
        expected,
        body.len() as u64,
        &body[..4],
        &mut gone,
    )
    .expect("the artifact is held, whatever happened to the rest of the body");
    assert_eq!(response.status, 201);
    assert!(response.body.contains("already holds"), "{}", response.body);
    assert_eq!(scratch.holds(), vec![name]);
}

/// **Retention, against a real filesystem**: a fourth version of one product
/// drops the lowest version, and another product's artifacts are untouched.
#[test]
fn a_fourth_version_of_a_product_prunes_the_lowest_and_says_so() {
    let scratch = Scratch::new("retention");
    // Another product first, so the assertion that it survives means something.
    scratch
        .put(&app("tempmon-rivet", "9.9.9"), b"tempmon")
        .expect("a good transfer");

    for (version, body) in [
        ("0.1.0", &b"one"[..]),
        ("0.2.0", &b"two"[..]),
        ("0.3.0", &b"three"[..]),
    ] {
        let response = scratch
            .put(&app("photo-index", version), body)
            .expect("a good transfer");
        assert!(
            !response.body.contains("pruned"),
            "{version} pruned something with three or fewer held: {}",
            response.body
        );
    }
    assert_eq!(scratch.holds().len(), 4, "{:?}", scratch.holds());

    // **Which one went, and not merely that one did.** Four directories created
    // in the same second have the same mtime, and a count that comes out right
    // says nothing about whether the selection did. `evictions` orders by
    // version, so the answer depends neither on the mtimes this test cannot
    // control nor on the digests of `b"one"`..`b"four"`, which bear no relation
    // to the versions — and so it can be asserted exactly.
    let fourth = scratch
        .put(&app("photo-index", "0.4.0"), b"four")
        .expect("a good transfer");
    assert!(
        fourth.body.contains(&app("photo-index", "0.1.0")),
        "the lowest version was not the one reported as pruned: {}",
        fourth.body
    );

    let held = scratch.holds();
    assert_eq!(held.len(), 4, "{held:?}");
    assert!(
        held.contains(&app("tempmon-rivet", "9.9.9")),
        "another product's only artifact was pruned: {held:?}"
    );
    for version in ["0.2.0", "0.3.0", "0.4.0"] {
        assert!(
            held.contains(&app("photo-index", version)),
            "{version} was evicted while a lower version survived: {held:?}"
        );
    }
    assert!(
        !held.contains(&app("photo-index", "0.1.0")),
        "the lowest version survived: {held:?}"
    );
    assert_eq!(
        held.iter()
            .filter(|name| name.starts_with("photo-index-"))
            .count(),
        artifact::KEEP,
        "{held:?}"
    );
}

/// Hex round-trips, so a refusal quotes what a caller can compare.
#[test]
fn a_digest_header_round_trips() {
    let bytes: Vec<u8> = (0u8..32).collect();
    let text = hex(&bytes);
    assert_eq!(text.len(), 64);
    assert_eq!(hex32(&text), Some(<[u8; 32]>::try_from(bytes.as_slice()).unwrap()));
    assert_eq!(hex32("nope"), None);
    assert_eq!(hex32(&"z".repeat(64)), None);
}

/// The response is a well-formed HTTP message whose length header is right —
/// wrong, and a client waits for bytes that never come.
#[test]
fn a_response_is_well_formed() {
    let response = Response {
        status: 201,
        reason: "Created",
        body: "a body\n".to_string(),
    };
    let text = response.render();
    let (head, body) = text.split_once("\r\n\r\n").expect("a head/body split");
    assert!(head.starts_with("HTTP/1.1 201 Created\r\n"), "{head}");
    assert!(head.contains(&format!("Content-Length: {}", body.len())), "{head}");
    assert_eq!(body, "a body\n");
}

/// The digest helper agrees with what a caller would compute, which is the
/// whole point of the header.
#[test]
fn the_digest_this_computes_is_the_one_a_caller_sends() {
    let body = b"rivet";
    let expected = digest_of(body);
    assert_eq!(expected.len(), 64);
    assert!(hex32(&expected).is_some());

}

/// **A chunked body is NOTICED**, so that it can be refused rather than handed
/// on with its framing still in it (`PLAN.md` §88.11). `git` chunks a request
/// larger than `http.postBuffer`, so this is a real push and not a curiosity.
#[test]
fn a_chunked_request_is_recognised_as_one() {
    let text = "POST /r.git/git-receive-pack HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n";
    let (request, _) = head(text).expect("well formed").expect("complete");
    assert!(request.chunked, "a chunked body was not noticed");
    assert_eq!(request.length, None, "a chunked request states no length");

    let plain = "PUT /payload/x.rvt HTTP/1.1\r\nContent-Length: 4\r\n\r\n";
    assert!(!head(plain).expect("well formed").expect("complete").0.chunked);
}

/// **A single `Content-Length` still parses exactly as before** — the
/// refusals below are about a SECOND framing claim, not about this endpoint's
/// ordinary shape.
#[test]
fn a_single_content_length_header_still_parses_normally() {
    let text = "PUT /payload/x.rvt HTTP/1.1\r\nRivet-Blake3: ab\r\nContent-Length: 9\r\n\r\nbody";
    let (request, _) = head(text).expect("well formed").expect("complete");
    assert_eq!(request.length, Some(9));
    assert!(!request.chunked);
}

/// **Two `Content-Length` headers that agree are refused too**, not trusted
/// because they matched. M4: nothing downstream of this parser should ever
/// have to decide which of two headers with the same name was the real one.
#[test]
fn two_content_length_headers_that_agree_are_refused() {
    let text =
        "PUT /payload/x.rvt HTTP/1.1\r\nRivet-Blake3: ab\r\nContent-Length: 9\r\nContent-Length: 9\r\n\r\nbody";
    let refusal = head(text).expect_err("two Content-Length headers must be refused");
    assert_eq!(refusal.status, 400, "{}", refusal.body);
    assert!(refusal.body.contains("Content-Length"), "{}", refusal.body);
}

/// **Two `Content-Length` headers that disagree are refused** — the classic
/// request-smuggling shape (RFC 9112 §6.3): a component in front honouring
/// one and this endpoint honouring the other would disagree about where the
/// body ends.
#[test]
fn two_content_length_headers_that_disagree_are_refused() {
    let text =
        "PUT /payload/x.rvt HTTP/1.1\r\nRivet-Blake3: ab\r\nContent-Length: 9\r\nContent-Length: 500\r\n\r\nbody";
    let refusal = head(text).expect_err("disagreeing Content-Length headers must be refused");
    assert_eq!(refusal.status, 400, "{}", refusal.body);
    assert!(refusal.body.contains('9') && refusal.body.contains("500"), "{}", refusal.body);
}

/// **`Transfer-Encoding` together with `Content-Length` is refused before a
/// byte of body is read** — the same two-framings shape `rivet-proxyd`
/// refuses. Today the digest check would eventually catch the damage; M4
/// closes it at the parser instead, before any body is streamed or staged.
#[test]
fn transfer_encoding_together_with_content_length_is_refused() {
    let text = "PUT /payload/x.rvt HTTP/1.1\r\nRivet-Blake3: ab\r\nContent-Length: 9\r\nTransfer-Encoding: chunked\r\n\r\nbody";
    let refusal = head(text).expect_err("Transfer-Encoding with Content-Length must be refused");
    assert_eq!(refusal.status, 400, "{}", refusal.body);
    assert!(refusal.body.contains("Transfer-Encoding"), "{}", refusal.body);
}

/// **None of the three refusals above stage anything** — they happen inside
/// [`parse_head`], which only ever reads the head, and `accept`/`receive` are
/// never reached for a request `parse_head` has already refused.
#[test]
fn a_refused_framing_never_reaches_accept_or_receive() {
    for text in [
        "PUT /payload/x.rvt HTTP/1.1\r\nContent-Length: 9\r\nContent-Length: 9\r\n\r\n",
        "PUT /payload/x.rvt HTTP/1.1\r\nContent-Length: 9\r\nContent-Length: 500\r\n\r\n",
        "PUT /payload/x.rvt HTTP/1.1\r\nContent-Length: 9\r\nTransfer-Encoding: chunked\r\n\r\n",
    ] {
        assert!(head(text).is_err(), "{text:?} was not refused at the head");
    }
}

// ---------------------------------------------------------------------------
// The door — no unverified artifact enters the store. ADR-052 §7.
//
// **The cryptography below is real**: BLAKE3 and Ed25519 out of rivetos-crypto,
// a `.rvt` sealed by rivet-core's own writer, and a trust store in
// rivet-core's own format on disk. What is not real is the machine: the store
// and the trust store are in a temporary directory rather than under `/state`
// and `/system`.
// ---------------------------------------------------------------------------

/// A signing key derived from a label — minted in memory, never written down.
fn signer(label: &[u8]) -> rivet_crypto::Ed25519Signer {
    let mut digest = rivet_crypto::Blake3::new();
    rivet_crypto::Ed25519Signer::from_seed(&digest.of(label))
}

const TRUSTED: &[u8] = b"rivet-mcpd bulk test: the key this machine trusts";
const STRANGER: &[u8] = b"rivet-mcpd bulk test: a key this machine has never seen";

/// A real signed application artifact, several members long.
fn signed(by: &[u8], product: &str, version: &str) -> Vec<u8> {
    signed_for(by, product, version, "aarch64")
}

/// [`signed`], built for `arch`.
fn signed_for(by: &[u8], product: &str, version: &str, arch: &str) -> Vec<u8> {
    use rivet_core::package::kind::Kind;
    use rivet_core::package::write::{Builder, NewMember};
    let mut digest = rivet_crypto::Blake3::new();
    let mut builder =
        Builder::new(product, version, arch, None, Kind::application()).expect("a header");
    let parts: Vec<(&str, Vec<u8>)> = vec![
        ("manifest.toml", b"kind = \"daemon\"\n".to_vec()),
        // Larger than one 64 KiB read, so the door's file source loops.
        ("program", (0..200_000usize).map(|i| (i % 251) as u8).collect()),
    ];
    for (i, (name, bytes)) in parts.iter().enumerate() {
        builder
            .add(
                NewMember::stored(name, u8::try_from(i).unwrap(), 0, bytes, &mut digest)
                    .expect("a member"),
            )
            .expect("room for it");
    }
    let slices: Vec<&[u8]> = parts.iter().map(|(_, b)| b.as_slice()).collect();
    builder
        .seal_in_memory(&mut digest, &mut signer(by), &slices)
        .expect("a sealed artifact")
}

/// A scratch store whose door reads a trust store holding [`TRUSTED`], entitled
/// to sign applications, from a file beside it.
fn guarded(label: &str) -> Scratch {
    use rivet_core::package::kind::Kind;
    use rivet_core::package::port::{MaySign, Sign as _};
    use rivet_core::package::trust_store::{Entry, TrustStore};
    let dir = std::env::temp_dir().join(format!("rivet-bulk-{}-{label}", std::process::id()));
    let trust = dir.join("trust");
    let arch = dir.join("arch");
    let scratch = Scratch::with_door(
        label,
        Box::new(TrustStoreDoor::at(trust.to_string_lossy(), arch.to_string_lossy())),
    );
    // What a Pi 4's image records (`rivet_core::package::arch`).
    std::fs::write(&arch, rivet_core::package::arch::render("aarch64").expect("a token"))
        .expect("the architecture record");
    let bytes = TrustStore::new(vec![Entry {
        key: signer(TRUSTED).public_key(),
        label: "test-apps".to_string(),
        may_sign: MaySign::these(&[Kind::application()]),
    }])
    .to_bytes()
    .expect("a store");
    std::fs::write(&trust, bytes).expect("the trust store");
    scratch
}

/// Nothing under `incoming/`, at any depth — a refusal leaves no directory.
fn incoming_is_empty(scratch: &Scratch) -> bool {
    std::fs::read_dir(scratch.store.incoming_dir())
        .map_or(true, |mut entries| entries.next().is_none())
}

/// **A trusted artifact is verified at the door and stored**, and the answer
/// says what was verified — by which key, for which kind.
#[test]
fn a_trusted_artifact_is_verified_at_the_door_and_stored() {
    let scratch = guarded("door-trusted");
    let body = signed(TRUSTED, "photo-index", "0.1.0");
    let name = app("photo-index", "0.1.0");

    let response = scratch.put(&name, &body).expect("a trusted artifact is admitted");
    assert_eq!(response.status, 201);
    assert!(response.body.contains("verified"), "{}", response.body);
    assert!(response.body.contains("\"test-apps\""), "{}", response.body);
    assert!(response.body.contains("application"), "{}", response.body);
    assert_eq!(scratch.holds(), vec![name]);
    assert!(incoming_is_empty(&scratch));
}

/// **The ruling, as a test.** An artifact signed by a key this machine does not
/// trust is refused at the door: the transfer was intact, the answer is a 422
/// that says so, and it NEVER appears in `objects/` or `incoming/` — so it holds
/// no retention slot and nothing can install it.
#[test]
fn an_untrusted_artifact_is_refused_at_the_door_and_never_stored() {
    let scratch = guarded("door-untrusted");
    let body = signed(STRANGER, "photo-index", "0.9.9");
    let name = app("photo-index", "0.9.9");

    let refusal = scratch.put(&name, &body).expect_err("an untrusted artifact must be refused");
    assert_eq!(refusal.status, 422, "{}", refusal.body);
    assert!(refusal.body.contains("REFUSED AT THE DOOR"), "{}", refusal.body);
    assert!(
        refusal.body.contains("not in this machine's trust set"),
        "the refusal must say WHY: {}",
        refusal.body
    );
    assert!(refusal.body.contains("transfer intact"), "{}", refusal.body);
    assert!(scratch.holds().is_empty(), "{:?}", scratch.holds());
    assert!(
        !std::path::Path::new(&scratch.store.objects_dir()).exists()
            || std::fs::read_dir(scratch.store.objects_dir())
                .expect("objects")
                .next()
                .is_none(),
        "objects/ holds something after a refusal"
    );
    assert!(incoming_is_empty(&scratch), "a refusal left something in incoming/");
}

/// **A genuine header over altered bytes is refused**, with the transfer digest
/// honest — the sender hashed the altered file — so the only thing that can
/// catch it is the member digests the door checks.
#[test]
fn a_signed_header_over_altered_members_is_refused_at_the_door() {
    let scratch = guarded("door-altered");
    let mut body = signed(TRUSTED, "photo-index", "0.1.0");
    let last = body.len() - 7;
    body[last] ^= 0x40;

    let refusal = scratch
        .put(&app("photo-index", "0.1.0"), &body)
        .expect_err("an altered member must be refused");
    assert_eq!(refusal.status, 422, "{}", refusal.body);
    assert!(refusal.body.contains("does not match the digest"), "{}", refusal.body);
    assert!(scratch.holds().is_empty(), "{:?}", scratch.holds());
    assert!(incoming_is_empty(&scratch));
}

/// **A machine that cannot read its trust store admits NOTHING** — refused as
/// the machine's fault, not the artifact's, and nothing stored.
#[test]
fn a_machine_whose_trust_store_cannot_be_read_admits_nothing() {
    let scratch = guarded("door-no-store");
    std::fs::remove_file(scratch.dir.join("trust")).expect("the trust store");
    let body = signed(TRUSTED, "photo-index", "0.1.0");

    let refusal = scratch
        .put(&app("photo-index", "0.1.0"), &body)
        .expect_err("no trust store, no admission");
    assert_eq!(refusal.status, 500, "{}", refusal.body);
    assert!(refusal.body.contains("could not verify"), "{}", refusal.body);
    assert!(refusal.body.contains("--no-trust"), "{}", refusal.body);
    assert!(scratch.holds().is_empty(), "{:?}", scratch.holds());
    assert!(incoming_is_empty(&scratch));
}

/// **An artifact built for another architecture is refused at the door** —
/// `PLAN-Feature-Set-2026-09-12` §3. Correctly signed, intact, and for a
/// machine that is not this one: until 2026-09-12 it was admitted and then
/// installed, and failed when it ran.
#[test]
fn an_artifact_for_another_architecture_is_refused_at_the_door_and_never_stored() {
    let scratch = guarded("door-wrong-arch");
    let body = signed_for(TRUSTED, "photo-index", "0.1.0", "armv6");
    let name = "photo-index-0.1.0-armv6-application.rvt";

    let refusal = scratch.put(name, &body).expect_err("an armv6 artifact on an aarch64 machine");
    assert_eq!(refusal.status, 422, "{}", refusal.body);
    assert!(refusal.body.contains("REFUSED AT THE DOOR"), "{}", refusal.body);
    assert!(refusal.body.contains("armv6"), "the refusal must name the artifact's arch: {}", refusal.body);
    assert!(refusal.body.contains("aarch64"), "the refusal must name this machine's: {}", refusal.body);
    assert!(scratch.holds().is_empty(), "{:?}", scratch.holds());
    assert!(incoming_is_empty(&scratch));
}

/// **An application whose name an image service already holds is refused at
/// the door and never stored** — `PLAN-Feature-Set-2026-09-12` §3,
/// `.rulings/` R-156 R10. Correctly signed and intact — the only thing wrong
/// with it is what it is named: without this check it would install and then
/// be refused every boot by `cmd/rivet-sysd`'s own discovery, with no
/// uninstall able to name the shadow.
#[test]
fn an_application_named_like_an_image_service_is_refused_at_the_door_and_never_stored() {
    let scratch = guarded("door-image-service-name");
    let body = signed(TRUSTED, rivet_core::service::MCPD_SERVICE_NAME, "0.1.0");
    let name = app(rivet_core::service::MCPD_SERVICE_NAME, "0.1.0");

    let refusal = scratch
        .put(&name, &body)
        .expect_err("an application named rivet-mcpd");
    assert_eq!(refusal.status, 422, "{}", refusal.body);
    assert!(refusal.body.contains("REFUSED AT THE DOOR"), "{}", refusal.body);
    assert!(
        refusal.body.contains(rivet_core::service::MCPD_SERVICE_NAME),
        "the refusal must name the clashing service: {}",
        refusal.body
    );
    assert!(scratch.holds().is_empty(), "{:?}", scratch.holds());
    assert!(incoming_is_empty(&scratch));
}

/// **An application named for something this project's own machinery already
/// means is refused at the door and never stored** — `.rulings/` R-210 D21.
/// `root` is a system identity, not a service, so this is the sibling of the
/// image-service test above rather than the same case: a different table, the
/// same door.
#[test]
fn an_application_named_for_a_reserved_name_is_refused_at_the_door_and_never_stored() {
    let scratch = guarded("door-reserved-name");
    let body = signed(TRUSTED, "root", "0.1.0");
    let name = app("root", "0.1.0");

    let refusal = scratch.put(&name, &body).expect_err("an application named root");
    assert_eq!(refusal.status, 422, "{}", refusal.body);
    assert!(refusal.body.contains("REFUSED AT THE DOOR"), "{}", refusal.body);
    assert!(refusal.body.contains("root"), "the refusal must name the clash: {}", refusal.body);
    assert!(scratch.holds().is_empty(), "{:?}", scratch.holds());
    assert!(incoming_is_empty(&scratch));
}

/// **A machine whose image recorded no architecture admits nothing** — the
/// machine's fault, a 500, and nothing stored. Defaulting to anything would be
/// a guess about what this machine executes.
#[test]
fn a_machine_with_no_architecture_record_admits_nothing() {
    let scratch = guarded("door-no-arch");
    std::fs::remove_file(scratch.dir.join("arch")).expect("the architecture record");
    let body = signed(TRUSTED, "photo-index", "0.1.0");

    let refusal = scratch
        .put(&app("photo-index", "0.1.0"), &body)
        .expect_err("no architecture record, no admission");
    assert_eq!(refusal.status, 500, "{}", refusal.body);
    assert!(refusal.body.contains(rivet_core::package::arch::PATH), "{}", refusal.body);
    assert!(scratch.holds().is_empty(), "{:?}", scratch.holds());
    assert!(incoming_is_empty(&scratch));
}

/// **A store written before the door existed can hold garbage, and offering it
/// again removes it.** The object is planted directly, as a pre-2026-09-12
/// endpoint would have stored it; a re-send is judged by the door like any
/// arrival, refused, and the held copy is gone — so no sweep is needed for a
/// stale artifact to stop holding a slot.
#[test]
fn an_unverified_artifact_already_held_is_removed_when_it_is_offered_again() {
    let scratch = guarded("door-held-garbage");
    let body = signed(STRANGER, "photo-index", "0.9.9");
    let name = app("photo-index", "0.9.9");
    let digest = artifact::Digest::new(&digest_of(&body)).expect("a digest");
    let object = scratch.store.object_path(&digest, &name);
    std::fs::create_dir_all(scratch.store.object_dir(&digest)).expect("an object directory");
    std::fs::write(&object, &body).expect("the planted object");
    assert_eq!(scratch.holds(), vec![name.clone()]);

    let refusal = scratch.put(&name, &body).expect_err("a held untrusted artifact is refused");
    assert_eq!(refusal.status, 422, "{}", refusal.body);
    assert!(refusal.body.contains("REMOVED"), "{}", refusal.body);
    assert!(scratch.holds().is_empty(), "{:?}", scratch.holds());
}

/// **But a held copy is never removed for the MACHINE's fault.** With the trust
/// store unreadable nothing is known about the artifact, so it stays.
#[test]
fn a_held_artifact_survives_a_door_that_cannot_decide() {
    let scratch = guarded("door-held-undecided");
    let body = signed(TRUSTED, "photo-index", "0.1.0");
    let name = app("photo-index", "0.1.0");
    scratch.put(&name, &body).expect("admitted while the store was readable");
    std::fs::remove_file(scratch.dir.join("trust")).expect("the trust store");

    let refusal = scratch.put(&name, &body).expect_err("nothing can be vouched for");
    assert_eq!(refusal.status, 500, "{}", refusal.body);
    assert_eq!(scratch.holds(), vec![name], "a held artifact was removed for the machine's fault");
}

/// **A trusted re-send still succeeds**, and now says the held copy verified.
#[test]
fn a_trusted_artifact_already_held_is_verified_again_and_answered_201() {
    let scratch = guarded("door-held-trusted");
    let body = signed(TRUSTED, "photo-index", "0.1.0");
    let name = app("photo-index", "0.1.0");
    scratch.put(&name, &body).expect("admitted");

    let again = scratch.put(&name, &body).expect("a re-send of a trusted artifact");
    assert_eq!(again.status, 201);
    assert!(again.body.contains("already holds"), "{}", again.body);
    assert!(again.body.contains("verified"), "{}", again.body);
    assert_eq!(scratch.holds(), vec![name]);
}

/// **A refusal at the door never costs a stored artifact its place.** Three
/// trusted versions are held; an untrusted, higher-numbered one is refused and
/// retention does not run for it — which was the whole of the defect: a
/// `0.9.9` that could not install used to keep one of the three.
#[test]
fn a_refused_high_version_takes_no_retention_slot() {
    let scratch = guarded("door-retention");
    for version in ["0.1.2", "0.1.3", "0.1.4"] {
        scratch
            .put(&app("hello-rivet", version), &signed(TRUSTED, "hello-rivet", version))
            .expect("a trusted version");
    }
    let refusal = scratch
        .put(&app("hello-rivet", "0.9.9"), &signed(STRANGER, "hello-rivet", "0.9.9"))
        .expect_err("the untrusted 0.9.9");
    assert_eq!(refusal.status, 422);
    assert!(!refusal.body.contains("pruned"), "{}", refusal.body);
    assert_eq!(
        scratch.holds(),
        vec![
            app("hello-rivet", "0.1.2"),
            app("hello-rivet", "0.1.3"),
            app("hello-rivet", "0.1.4"),
        ]
    );
}

/// A door that refuses leaves nothing, whatever it refused for — the order
/// around the decision, independent of cryptography.
#[test]
fn whatever_the_door_refuses_leaves_nothing_behind() {
    let scratch = Scratch::with_door(
        "door-recording-refuses",
        Box::new(Recording {
            refuse: true,
            ..Recording::default()
        }),
    );
    let refusal = scratch
        .put(&app("photo-index", "0.1.0"), b"any bytes at all")
        .expect_err("refused");
    assert_eq!(refusal.status, 422);
    assert!(scratch.holds().is_empty());
    assert!(incoming_is_empty(&scratch));
}

// ---------------------------------------------------------------------------
// Two uploads of one digest, forced to overlap — B17. Every test here uses
// real threads and real synchronisation (channels, never a sleep) to force
// the exact interleaving the defect needs, rather than hoping the scheduler
// produces it.
// ---------------------------------------------------------------------------

/// A door that admits everything and holds no state — `Send`, so a fresh one
/// can be built inside whichever thread wants it.
struct AlwaysAdmit;

impl Door for AlwaysAdmit {
    fn admit(&self, _path: &str, _length: u64) -> Result<String, Refusal> {
        Ok("\x20 verified by a test door that admits everything\n".to_string())
    }
}

/// A door that signals `entered` and then blocks on `release` before
/// admitting — so a transfer can be held open exactly between its digest
/// check and its publish, which is the window [`receive_onto`] calls `door`
/// in.
struct PausingAdmit {
    entered: std::sync::mpsc::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

impl Door for PausingAdmit {
    fn admit(&self, _path: &str, _length: u64) -> Result<String, Refusal> {
        let _ = self.entered.send(());
        let _ = self.release.recv();
        Ok("\x20 verified by a test door paused for a concurrency test\n".to_string())
    }
}

/// A body that hands out `chunks` in order and, the first time it is asked
/// for any, signals `entered` and blocks on `release` before returning that
/// first slice. Once `chunks` is exhausted it reports `Ok(0)` — end of
/// stream — which is what a shorter-than-declared `chunks` turns into a
/// connection that vanished mid-body.
struct ControlledBody {
    chunks: Vec<u8>,
    sent: usize,
    paused: bool,
    entered: std::sync::mpsc::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

impl ControlledBody {
    fn call(&mut self, into: &mut [u8]) -> std::io::Result<usize> {
        if self.sent >= self.chunks.len() {
            return Ok(0);
        }
        let n = into.len().min(self.chunks.len() - self.sent);
        into[..n].copy_from_slice(&self.chunks[self.sent..self.sent + n]);
        self.sent += n;
        if !self.paused {
            self.paused = true;
            let _ = self.entered.send(());
            let _ = self.release.recv();
        }
        Ok(n)
    }
}

/// A body that hands out `body` in order with no pause, signalling `entered`
/// the first time it is asked for any of it — proof, from outside, of the
/// moment a transfer was allowed to start reading.
struct SignallingBody {
    body: Vec<u8>,
    sent: usize,
    entered: std::sync::mpsc::Sender<()>,
}

impl SignallingBody {
    fn call(&mut self, into: &mut [u8]) -> std::io::Result<usize> {
        if self.sent == 0 {
            let _ = self.entered.send(());
        }
        let n = into.len().min(self.body.len() - self.sent);
        into[..n].copy_from_slice(&self.body[self.sent..self.sent + n]);
        self.sent += n;
        Ok(n)
    }
}

/// A generous bound for "this must finish" — never hit when the fix works;
/// only there so a real deadlock reports as a failure instead of hanging
/// `cargo test` forever.
const PATIENCE: std::time::Duration = std::time::Duration::from_secs(5);

/// A bound for "this must NOT have happened yet". Short, so the suite stays
/// fast; long enough that a loaded machine does not make it flaky. Every use
/// below is checking an ABSENCE — that a second transfer has not been let
/// read a body byte — which is exactly what the fix is supposed to prevent
/// for as long as the first transfer holds its claim.
const BLOCK_CHECK: std::time::Duration = std::time::Duration::from_millis(500);

/// **Two uploads of the same digest and the same member name, forced to
/// overlap, do not corrupt each other.**
///
/// Before the fix, `B`'s `File::create` on the identical temporary path
/// truncates what `A` had already written the instant `B` is allowed to run.
/// `B` is proven not to have read a single body byte while `A` is paused —
/// that is the exclusion the fix adds — and then the bytes actually on disk
/// afterward are checked as corroborating evidence.
#[test]
fn concurrent_uploads_of_the_same_digest_and_name_do_not_corrupt_each_other() {
    let scratch = Scratch::new("race-same-name");
    let name = app("photo-index", "0.1.0");
    let body: Vec<u8> = (0..200_000u32).map(|i| u8::try_from(i % 251).unwrap_or(0)).collect();
    let expected = hex32(&digest_of(&body)).expect("64 hex characters");
    let length = body.len() as u64;

    let (a_entered_tx, a_entered_rx) = std::sync::mpsc::channel::<()>();
    let (a_release_tx, a_release_rx) = std::sync::mpsc::channel::<()>();
    let (a_done_tx, a_done_rx) = std::sync::mpsc::channel::<Result<Response, Response>>();
    let store_a = scratch.store.clone();
    let name_a = name.clone();
    let body_a = body.clone();
    std::thread::spawn(move || {
        let mut reader = ControlledBody {
            chunks: body_a,
            sent: 0,
            paused: false,
            entered: a_entered_tx,
            release: a_release_rx,
        };
        let mut more = move |into: &mut [u8]| reader.call(into);
        let result = receive(&store_a, &AlwaysAdmit, &name_a, expected, length, &[], &mut more);
        let _ = a_done_tx.send(result);
    });
    a_entered_rx.recv_timeout(PATIENCE).expect("A did not reach its controlled pause");

    let (b_entered_tx, b_entered_rx) = std::sync::mpsc::channel::<()>();
    let (b_done_tx, b_done_rx) = std::sync::mpsc::channel::<Result<Response, Response>>();
    let store_b = scratch.store.clone();
    let name_b = name.clone();
    let body_b = body.clone();
    std::thread::spawn(move || {
        let mut reader = SignallingBody { body: body_b, sent: 0, entered: b_entered_tx };
        let mut more = move |into: &mut [u8]| reader.call(into);
        let result = receive(&store_b, &AlwaysAdmit, &name_b, expected, length, &[], &mut more);
        let _ = b_done_tx.send(result);
    });

    assert!(
        matches!(b_entered_rx.recv_timeout(BLOCK_CHECK), Err(std::sync::mpsc::RecvTimeoutError::Timeout)),
        "the second upload read a body byte while the first still held the same digest — the \
         two were not exclusive"
    );

    a_release_tx.send(()).expect("release A");
    let ra = a_done_rx.recv_timeout(PATIENCE).expect("A did not finish").expect("A's transfer");
    let rb = b_done_rx.recv_timeout(PATIENCE).expect("B did not finish").expect("B's transfer");
    assert_eq!(ra.status, 201, "{}", ra.body);
    assert_eq!(rb.status, 201, "{}", rb.body);

    assert_eq!(scratch.holds(), vec![name.clone()]);
    let digest = artifact::Digest::new(&digest_of(&body)).expect("a digest");
    let path = scratch.store.object_path(&digest, &name);
    assert_eq!(std::fs::read(&path).expect("the bytes"), body, "the stored bytes were corrupted");
    assert!(
        !std::path::Path::new(&scratch.store.incoming_for(&digest)).exists(),
        "a finished pair of transfers left an incoming directory behind"
    );
}

/// **Two uploads of the same digest under different member names, forced to
/// overlap, never expose a partial member.**
///
/// Before the fix, whichever finishes first can rename the shared
/// `incoming/<digest>/` directory into the store while the other's
/// differently-named member is still being written inside what was, until
/// that rename, the very same directory — so the store holds two files
/// where the module's own contract says one. Both uploads carry identical
/// bytes, since a digest is a hash of content: that is what makes this a
/// legitimate race (two operators pushing the same artifact under two
/// names) rather than a forged collision.
#[test]
fn concurrent_uploads_of_the_same_digest_under_different_names_never_expose_a_partial_member() {
    let scratch = Scratch::new("race-diff-name");
    let name_first = app("photo-index", "0.1.0");
    let name_second = app("photo-index-mirror", "0.1.0");
    let body: Vec<u8> = (0..200_000u32).map(|i| u8::try_from(i % 197).unwrap_or(0)).collect();
    let expected = hex32(&digest_of(&body)).expect("64 hex characters");
    let length = body.len() as u64;

    let (a_entered_tx, a_entered_rx) = std::sync::mpsc::channel::<()>();
    let (a_release_tx, a_release_rx) = std::sync::mpsc::channel::<()>();
    let (a_done_tx, a_done_rx) = std::sync::mpsc::channel::<Result<Response, Response>>();
    let store_a = scratch.store.clone();
    let name_a = name_first.clone();
    let body_a = body.clone();
    std::thread::spawn(move || {
        let mut reader = ControlledBody {
            chunks: body_a,
            sent: 0,
            paused: false,
            entered: a_entered_tx,
            release: a_release_rx,
        };
        let mut more = move |into: &mut [u8]| reader.call(into);
        let result = receive(&store_a, &AlwaysAdmit, &name_a, expected, length, &[], &mut more);
        let _ = a_done_tx.send(result);
    });
    a_entered_rx.recv_timeout(PATIENCE).expect("A did not reach its controlled pause");

    let (b_entered_tx, b_entered_rx) = std::sync::mpsc::channel::<()>();
    let (b_done_tx, b_done_rx) = std::sync::mpsc::channel::<Result<Response, Response>>();
    let store_b = scratch.store.clone();
    let name_b = name_second.clone();
    let body_b = body.clone();
    std::thread::spawn(move || {
        let mut reader = SignallingBody { body: body_b, sent: 0, entered: b_entered_tx };
        let mut more = move |into: &mut [u8]| reader.call(into);
        let result = receive(&store_b, &AlwaysAdmit, &name_b, expected, length, &[], &mut more);
        let _ = b_done_tx.send(result);
    });

    assert!(
        matches!(b_entered_rx.recv_timeout(BLOCK_CHECK), Err(std::sync::mpsc::RecvTimeoutError::Timeout)),
        "the second upload read a body byte while the first still held the same digest — the \
         two were not exclusive"
    );

    a_release_tx.send(()).expect("release A");
    let ra = a_done_rx.recv_timeout(PATIENCE).expect("A did not finish").expect("A's transfer");
    let rb = b_done_rx.recv_timeout(PATIENCE).expect("B did not finish").expect("B's transfer");
    assert_eq!(ra.status, 201, "{}", ra.body);
    assert_eq!(rb.status, 201, "{}", rb.body);

    // Whichever name won, the store holds exactly one object directory for
    // this digest, with exactly one file in it — never a second, differently
    // named member left behind by the loser.
    let held = scratch.holds();
    assert_eq!(held.len(), 1, "{held:?}");
    assert!(held[0] == name_first || held[0] == name_second, "{held:?}");
    let digest = artifact::Digest::new(&digest_of(&body)).expect("a digest");
    let path = scratch.store.object_path(&digest, &held[0]);
    assert_eq!(std::fs::read(&path).expect("the bytes"), body, "the stored bytes were corrupted");
    assert!(
        !std::path::Path::new(&scratch.store.incoming_for(&digest)).exists(),
        "a finished pair of transfers left an incoming directory behind"
    );
}

/// **When one of two concurrent uploads of a digest disconnects mid-body, the
/// other still lands cleanly.**
///
/// Before the fix, `A`'s failure cleanup removes the whole shared
/// `incoming/<digest>/` directory — which, without exclusion, could be
/// removed out from under `B` while `B` is still writing into it. With the
/// fix, `B` cannot even start until `A`'s claim (including its cleanup) has
/// fully released, so `B` always starts into a digest that is either absent
/// (A failed) or published (A does not, in this test) — never into a
/// half-cleaned one.
#[test]
fn when_one_concurrent_upload_of_a_digest_disconnects_the_other_still_lands_cleanly() {
    let scratch = Scratch::new("race-disconnect");
    let name = app("photo-index", "0.1.0");
    let body: Vec<u8> = (0..200_000u32).map(|i| u8::try_from(i % 233).unwrap_or(0)).collect();
    let expected = hex32(&digest_of(&body)).expect("64 hex characters");
    let length = body.len() as u64;
    let cut = body[..body.len() / 3].to_vec();

    let (a_entered_tx, a_entered_rx) = std::sync::mpsc::channel::<()>();
    let (a_release_tx, a_release_rx) = std::sync::mpsc::channel::<()>();
    let (a_done_tx, a_done_rx) = std::sync::mpsc::channel::<Result<Response, Response>>();
    let store_a = scratch.store.clone();
    let name_a = name.clone();
    std::thread::spawn(move || {
        let mut reader = ControlledBody {
            chunks: cut,
            sent: 0,
            paused: false,
            entered: a_entered_tx,
            release: a_release_rx,
        };
        let mut more = move |into: &mut [u8]| reader.call(into);
        let result = receive(&store_a, &AlwaysAdmit, &name_a, expected, length, &[], &mut more);
        let _ = a_done_tx.send(result);
    });
    a_entered_rx.recv_timeout(PATIENCE).expect("A did not reach its controlled pause");

    let (b_entered_tx, b_entered_rx) = std::sync::mpsc::channel::<()>();
    let (b_done_tx, b_done_rx) = std::sync::mpsc::channel::<Result<Response, Response>>();
    let store_b = scratch.store.clone();
    let name_b = name.clone();
    let body_b = body.clone();
    std::thread::spawn(move || {
        let mut reader = SignallingBody { body: body_b, sent: 0, entered: b_entered_tx };
        let mut more = move |into: &mut [u8]| reader.call(into);
        let result = receive(&store_b, &AlwaysAdmit, &name_b, expected, length, &[], &mut more);
        let _ = b_done_tx.send(result);
    });

    assert!(
        matches!(b_entered_rx.recv_timeout(BLOCK_CHECK), Err(std::sync::mpsc::RecvTimeoutError::Timeout)),
        "the second upload read a body byte while the first still held the same digest — the \
         two were not exclusive"
    );

    a_release_tx.send(()).expect("release A");
    let ra = a_done_rx.recv_timeout(PATIENCE).expect("A did not finish");
    let rb = b_done_rx.recv_timeout(PATIENCE).expect("B did not finish");

    let refusal = ra.expect_err("a connection that vanished mid-body must be refused");
    assert_eq!(refusal.status, 400, "{}", refusal.body);
    let ok = rb.expect("B's transfer must still succeed once A's failure has cleaned up");
    assert_eq!(ok.status, 201, "{}", ok.body);

    assert_eq!(scratch.holds(), vec![name.clone()]);
    let digest = artifact::Digest::new(&digest_of(&body)).expect("a digest");
    let path = scratch.store.object_path(&digest, &name);
    assert_eq!(std::fs::read(&path).expect("the bytes"), body);
    assert!(
        !std::path::Path::new(&scratch.store.incoming_for(&digest)).exists(),
        "A's failed transfer, or B's successful one, left an incoming directory behind"
    );
}

/// **A second upload of a digest waits through the first being paused
/// between its digest check and its publish rename.**
///
/// This is the narrowest window the defect names: the body is already fully
/// streamed and hashed, so a lock that only covered the streaming loop would
/// not reach it. `A`'s DOOR is what pauses here, not its body, so that by the
/// time `B` is allowed to run, `A` has already matched the digest and is
/// waiting only on the door's answer before renaming into the store.
#[test]
fn a_second_upload_of_a_digest_waits_through_the_first_paused_between_verify_and_publish() {
    let scratch = Scratch::new("race-verify-publish");
    let name = app("photo-index", "0.1.0");
    let body: Vec<u8> = (0..50_000u32).map(|i| u8::try_from(i % 179).unwrap_or(0)).collect();
    let expected = hex32(&digest_of(&body)).expect("64 hex characters");
    let length = body.len() as u64;

    let (a_entered_tx, a_entered_rx) = std::sync::mpsc::channel::<()>();
    let (a_release_tx, a_release_rx) = std::sync::mpsc::channel::<()>();
    let (a_done_tx, a_done_rx) = std::sync::mpsc::channel::<Result<Response, Response>>();
    let store_a = scratch.store.clone();
    let name_a = name.clone();
    let body_a = body.clone();
    std::thread::spawn(move || {
        let door = PausingAdmit { entered: a_entered_tx, release: a_release_rx };
        let mut sent = 0usize;
        let mut more = move |into: &mut [u8]| -> std::io::Result<usize> {
            let n = into.len().min(body_a.len() - sent);
            into[..n].copy_from_slice(&body_a[sent..sent + n]);
            sent += n;
            Ok(n)
        };
        let result = receive(&store_a, &door, &name_a, expected, length, &[], &mut more);
        let _ = a_done_tx.send(result);
    });
    a_entered_rx.recv_timeout(PATIENCE).expect("A did not reach the door");

    let (b_entered_tx, b_entered_rx) = std::sync::mpsc::channel::<()>();
    let (b_done_tx, b_done_rx) = std::sync::mpsc::channel::<Result<Response, Response>>();
    let store_b = scratch.store.clone();
    let name_b = name.clone();
    let body_b = body.clone();
    std::thread::spawn(move || {
        let mut reader = SignallingBody { body: body_b, sent: 0, entered: b_entered_tx };
        let mut more = move |into: &mut [u8]| reader.call(into);
        let result = receive(&store_b, &AlwaysAdmit, &name_b, expected, length, &[], &mut more);
        let _ = b_done_tx.send(result);
    });

    assert!(
        matches!(b_entered_rx.recv_timeout(BLOCK_CHECK), Err(std::sync::mpsc::RecvTimeoutError::Timeout)),
        "the second upload started while the first was paused between its digest check and its \
         publish — the two were not exclusive"
    );

    a_release_tx.send(()).expect("release A's door");
    let ra = a_done_rx.recv_timeout(PATIENCE).expect("A did not finish").expect("A's transfer");
    let rb = b_done_rx.recv_timeout(PATIENCE).expect("B did not finish").expect("B's transfer");
    assert_eq!(ra.status, 201, "{}", ra.body);
    assert_eq!(rb.status, 201, "{}", rb.body);
    assert!(rb.body.contains("already holds"), "{}", rb.body);

    assert_eq!(scratch.holds(), vec![name.clone()]);
    let digest = artifact::Digest::new(&digest_of(&body)).expect("a digest");
    let path = scratch.store.object_path(&digest, &name);
    assert_eq!(std::fs::read(&path).expect("the bytes"), body);
    assert!(!std::path::Path::new(&scratch.store.incoming_for(&digest)).exists());
}

/// **A link planted where a transfer is built is not written through.** A
/// store an application can write — a fleet server's repository store before
/// its directories were kept root's — let that application put a link at
/// `incoming/<digest>`, and root then created the arriving file wherever it
/// pointed. The transfer is built in a directory this daemon made itself.
#[cfg(unix)]
#[test]
fn a_link_planted_where_a_transfer_is_built_is_not_written_through() {
    let scratch = Scratch::new("planted-incoming");
    let outside = scratch.dir.join("outside");
    std::fs::create_dir_all(&outside).expect("outside");
    let body = b"an artifact planted against";
    let digest = artifact::Digest::new(&digest_of(body)).expect("a digest");
    std::fs::create_dir_all(scratch.store.incoming_dir()).expect("incoming");
    std::os::unix::fs::symlink(&outside, scratch.store.incoming_for(&digest)).expect("the planted link");

    let response = scratch.put(&app("planted", "0.1.0"), body).expect("the transfer still lands");
    assert_eq!(response.status, 201, "{}", response.body);
    assert_eq!(
        std::fs::read_dir(&outside).expect("outside").count(),
        0,
        "the arriving file was created through the planted link"
    );
    assert_eq!(scratch.holds(), vec![app("planted", "0.1.0")]);
}

/// **A store renamed aside mid-transfer, with a link planted under its name,
/// is still the store written.** The store is opened once and every entry
/// below it is made relative to that descriptor, so the swap redirects
/// nothing: the object lands in the real directory and nothing is created
/// where the link points.
#[cfg(unix)]
#[test]
fn a_store_swapped_for_a_link_mid_transfer_is_not_written_through() {
    let scratch = Scratch::new("swapped-root");
    let outside = scratch.dir.join("outside");
    std::fs::create_dir_all(&outside).expect("outside");
    let aside = scratch.dir.join("aside");
    let root = std::path::PathBuf::from(scratch.store.root());
    let body: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    let expected = hex32(&digest_of(&body)).expect("64 hex characters");
    let name = app("swapped", "0.1.0");

    let mut at = 0usize;
    let mut swapped = false;
    let mut feed = |into: &mut [u8]| -> std::io::Result<usize> {
        if !swapped {
            swapped = true;
            std::fs::rename(&root, &aside).expect("the store renamed aside");
            std::os::unix::fs::symlink(&outside, &root).expect("the planted link");
        }
        let n = into.len().min(body.len() - at);
        into[..n].copy_from_slice(&body[at..at + n]);
        at += n;
        Ok(n)
    };
    let answer = receive(&scratch.store, &AlwaysAdmit, &name, expected, body.len() as u64, &[], &mut feed);

    assert_eq!(
        std::fs::read_dir(&outside).expect("outside").count(),
        0,
        "something was created through the link planted in the store's place: {answer:?}"
    );
    let digest = artifact::Digest::new(&digest_of(&body)).expect("a digest");
    let landed = aside.join("objects").join(digest.as_str()).join(&name);
    assert_eq!(std::fs::read(&landed).expect("the object, in the real store"), body);
}

