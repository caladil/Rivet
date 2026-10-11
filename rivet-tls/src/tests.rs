//! **What the trust model does, proved rather than asserted.**
//!
//! Two of these matter more than the rest. `a_long_expired_certificate_...` is
//! `ADR-035` §7.1's requirement that the machine *say* whether it reads expiry
//! — this is the saying, in executable form. And the two `not_pinned` tests are
//! the trust model itself: a perfectly valid certificate is refused because the
//! key inside it was not expected.

// A test that cannot fail loudly is not a test.
//
// **File-scope rather than inside a `mod tests`**, because this whole file
// IS the test module - `lib.rs` declares it as `#[cfg(test)] mod tests;`.
// That shape is why it never got the block every inline test module in the
// workspace carries, and why it held 32 of the 287 warnings a lint run
// could not report while clippy was missing from the container.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::io::{Read, Write};

use super::*;

/// Mint what `cmd/cargo-rivet` will mint: an Ed25519 self-signed leaf, no CA,
/// no chain. `expired` closes the validity window in 1980.
fn mint(name: &str, expired: bool) -> Identity {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).expect("an Ed25519 pair");
    let mut params = rcgen::CertificateParams::new(vec![name.to_string()]).expect("params");
    params.is_ca = rcgen::IsCa::NoCa;
    if expired {
        params.not_before = rcgen::date_time_ymd(1975, 1, 1);
        params.not_after = rcgen::date_time_ymd(1980, 1, 1);
    }
    let cert = params.self_signed(&key).expect("a self-signed leaf");
    let der = key.serialize_der();
    // **rcgen hands back PKCS#8 v2, not v1.** v2 appends the PUBLIC key after
    // the private one, so the last thirty-two bytes are the wrong half — and
    // taking them produced `KeyMismatch` from rustls, which checks that a
    // certificate and its key agree before it will serve them. The seed
    // follows the `04 22 04 20` that opens the privateKey OCTET STRING.
    let at = der
        .windows(4)
        .position(|w| w == [0x04, 0x22, 0x04, 0x20])
        .expect("PKCS#8 for Ed25519 wraps the seed in 04 22 04 20")
        + 4;
    let mut seed = [0u8; KEY_BYTES];
    seed.copy_from_slice(&der[at..at + KEY_BYTES]);
    Identity { certificate: cert.der().to_vec(), seed }
}

/// Drive a client and a server against each other in memory until both stop
/// handshaking, or one of them refuses.
fn handshake(
    client: &mut rustls::ClientConnection,
    server: &mut rustls::ServerConnection,
) -> Result<(), rustls::Error> {
    for _ in 0..32 {
        let mut buf = Vec::new();
        client.write_tls(&mut buf).expect("an in-memory write");
        if !buf.is_empty() {
            server.read_tls(&mut buf.as_slice()).expect("an in-memory read");
            server.process_new_packets()?;
        }
        let mut buf = Vec::new();
        server.write_tls(&mut buf).expect("an in-memory write");
        if !buf.is_empty() {
            client.read_tls(&mut buf.as_slice()).expect("an in-memory read");
            client.process_new_packets()?;
        }
        if !client.is_handshaking() && !server.is_handshaking() {
            return Ok(());
        }
    }
    Ok(())
}

/// A board and an operator, each pinning whatever the caller says, wired to
/// each other.
fn connected(
    board: &Identity,
    operator: &Identity,
    client_pins: Vec<Pin>,
    server_pins: Vec<Pin>,
) -> (rustls::ClientConnection, rustls::ServerConnection) {
    let p = provider();
    let s = server_config(board, server_pins, p.clone()).expect("a server config");
    let c = client_config(operator, client_pins, p).expect("a client config");
    let server = rustls::ServerConnection::new(Arc::new(s)).expect("a server connection");
    let client = rustls::ClientConnection::new(
        Arc::new(c),
        rustls::pki_types::ServerName::try_from("board").expect("a name"),
    )
    .expect("a client connection");
    (client, server)
}

fn key_of(id: &Identity) -> Pin {
    id.public_key().expect("a minted leaf carries an Ed25519 key")
}

/// The PKCS#8 constant is right, and `rustls` is the judge of that rather than
/// a byte comparison of the constant against itself.
#[test]
fn a_seed_becomes_a_private_key_rustls_will_sign_with() {
    let key = provider()
        .key_provider
        .load_private_key(PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pkcs8_from_seed(
            &[7u8; KEY_BYTES],
        ))))
        .expect("the PKCS#8 prefix produces a key rustls accepts");
    assert_eq!(key.algorithm(), rustls::SignatureAlgorithm::ED25519);
}

/// The search finds the key a real certificate actually carries, and finds a
/// different one for a different certificate.
#[test]
fn the_public_key_is_found_in_a_certificate_that_was_really_minted() {
    let found = key_of(&mint("board", false));
    assert_ne!(found, [0u8; KEY_BYTES]);
    assert_ne!(found, key_of(&mint("board", false)), "the same key came back twice");
}

/// A certificate this crate did not mint, and could not read, yields nothing
/// rather than garbage.
#[test]
fn a_buffer_with_no_ed25519_spki_in_it_yields_no_key() {
    assert_eq!(public_key_in_certificate(&[0u8; 400]), None);
    assert_eq!(public_key_in_certificate(&[]), None);
}

/// **Mutual, and both ends really did authenticate.** `ADR-025` §2.
#[test]
fn two_peers_that_pin_each_other_complete_a_handshake_and_carry_data() {
    let board = mint("board", false);
    let operator = mint("operator", false);
    let (mut client, mut server) =
        connected(&board, &operator, vec![key_of(&board)], vec![key_of(&operator)]);

    handshake(&mut client, &mut server).expect("a handshake between pinned peers");
    assert!(!client.is_handshaking(), "the client never finished");
    assert!(!server.is_handshaking(), "the server never finished");
    assert!(
        server.peer_certificates().is_some_and(|c| !c.is_empty()),
        "the server accepted a client that presented no certificate at all"
    );

    client.writer().write_all(b"ping").expect("a write");
    let mut wire = Vec::new();
    client.write_tls(&mut wire).expect("a flush");
    server.read_tls(&mut wire.as_slice()).expect("a read");
    server.process_new_packets().expect("records");
    let mut got = [0u8; 8];
    let n = server.reader().read(&mut got).expect("the payload");
    assert_eq!(&got[..n], b"ping");
}

/// **A client the server does not know is refused**, however good its
/// certificate. This is the trust model working rather than failing.
#[test]
fn a_client_whose_key_is_not_pinned_is_refused_by_the_server() {
    let board = mint("board", false);
    let operator = mint("operator", false);
    let (mut client, mut server) =
        connected(&board, &operator, vec![key_of(&board)], vec![[9u8; KEY_BYTES]]);
    let err = handshake(&mut client, &mut server)
        .expect_err("a client nobody pinned completed a handshake");
    assert!(format!("{err}").contains("not one this machine was told to expect"), "{err}");
}

/// And symmetrically: a board the operator does not recognise is refused.
#[test]
fn a_server_whose_key_is_not_pinned_is_refused_by_the_client() {
    let board = mint("board", false);
    let operator = mint("operator", false);
    let (mut client, mut server) =
        connected(&board, &operator, vec![[9u8; KEY_BYTES]], vec![key_of(&operator)]);
    let err = handshake(&mut client, &mut server)
        .expect_err("a server nobody pinned completed a handshake");
    assert!(format!("{err}").contains("not one this machine was told to expect"), "{err}");
}

/// **`ADR-035` §7.1 requires the machine to say whether expiry is ignored or
/// set far out. It is IGNORED, and this is the proof rather than the claim.**
///
/// A certificate whose validity window closed in 1980 — decades before any
/// clock this hardware could hold, wrong or right — completes a handshake,
/// because the only question asked of it is whether its key is pinned.
#[test]
fn a_long_expired_certificate_is_accepted_because_expiry_is_not_a_trust_condition() {
    let board = mint("board", true);
    let operator = mint("operator", false);
    let (mut client, mut server) =
        connected(&board, &operator, vec![key_of(&board)], vec![key_of(&operator)]);
    handshake(&mut client, &mut server)
        .expect("a certificate that expired in 1980 was refused, so expiry IS being read");
    assert!(!client.is_handshaking(), "the client never finished");
}

/// The name in the certificate is not consulted either — the pin is the
/// identity, and a name would be a second authority for one question.
#[test]
fn the_certificates_name_is_not_checked_against_the_one_dialled() {
    let board = mint("something-else-entirely", false);
    let operator = mint("operator", false);
    let (mut client, mut server) =
        connected(&board, &operator, vec![key_of(&board)], vec![key_of(&operator)]);
    handshake(&mut client, &mut server).expect("the dialled name is not a trust condition");
    assert!(!client.is_handshaking());
}

/// An empty pin set is refused when the config is built, not when a peer
/// arrives.
#[test]
fn nothing_pinned_is_refused_when_the_config_is_built() {
    let id = mint("board", false);
    assert_eq!(server_config(&id, Vec::new(), provider()).err(), Some(Error::NoPins));
    assert_eq!(client_config(&id, Vec::new(), provider()).err(), Some(Error::NoPins));
}

/// A certificate with no Ed25519 key in it is refused by name rather than as a
/// generic mismatch.
#[test]
fn a_certificate_carrying_no_ed25519_key_is_refused_as_such() {
    let peer = PinnedPeer::new(vec![[1u8; KEY_BYTES]], provider()).expect("a verifier");
    let nonsense = CertificateDer::from(vec![0x30, 0x03, 0x02, 0x01, 0x00]);
    assert_eq!(peer.check(&nonsense, 0), Err(Error::NotEd25519));
}

/// A chain is refused rather than ignored — `ADR-035` §7.2.
#[test]
fn intermediates_are_refused_because_there_is_nothing_to_chain_to() {
    let id = mint("board", false);
    let peer = PinnedPeer::new(vec![key_of(&id)], provider()).expect("a verifier");
    let leaf = CertificateDer::from(id.certificate.clone());
    assert_eq!(peer.check(&leaf, 0), Ok(()), "the leaf on its own is fine");
    assert_eq!(peer.check(&leaf, 2), Err(Error::ChainPresented(2)));
}

/// **The seed never reaches a log.** One `derive(Debug)` away from a private
/// key in the journal.
#[test]
fn debugging_an_identity_does_not_print_the_seed() {
    let id = Identity { certificate: vec![1, 2, 3], seed: [0xAB; KEY_BYTES] };
    let shown = format!("{id:?}");
    assert!(shown.contains("<not shown>"), "{shown}");
    assert!(!shown.contains("ab"), "the seed reached Debug output: {shown}");
    assert!(!shown.contains("171"), "the seed reached Debug output: {shown}");
}

/// Every refusal names what it means and cites where the rule comes from,
/// because these arrive on a headless board where the message is all there is.
#[test]
fn every_refusal_says_what_it_means_and_cites_the_rule() {
    for e in [
        Error::NotEd25519,
        Error::NotPinned,
        Error::ChainPresented(1),
        Error::NoCertificate,
        Error::NoPins,
    ] {
        let said = e.to_string();
        assert!(said.len() > 80, "{e:?} says too little: {said}");
        assert!(said.contains("ADR-0"), "{e:?} cites no rule: {said}");
    }
}

/// A pin file is a list of who may control the machine, so it has to be
/// reviewable: comments and blank lines are how a human says whose key is
/// whose.
#[test]
fn a_pin_file_carries_comments_and_blank_lines() {
    let text = "\
# Someone's workstation
aabbccddeeff00112233445566778899aabbccddeeff001122334455667788ff

# the fleet server
00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff  # x86
";
    let pins = parse_pins(text).expect("a readable pin file");
    assert_eq!(pins.len(), 2);
    assert_eq!(pins[0][0], 0xaa);
    assert_eq!(pins[1][0], 0x00);
}

/// **The shape a board's file actually has, with no trailing newline.**
///
/// `apply edit --append` adds *exactly* the text it is given — no newline is
/// added and none is removed — so a pin file grown one key at a time over the
/// control plane ends without one. That is the file `cmd/rivet-mcpd` reads at
/// **startup**, and [`super::tls_config`]'s refusal on an unreadable one
/// happens before a socket is bound: a machine whose pin file will not parse
/// does not come back, and the operator finds out by not being able to connect.
///
/// So the exact bytes are asserted rather than a tidy fixture. The failure this
/// guards against is real and was met on 2026-09-06: appending a key **without**
/// a leading newline concatenates it onto the previous one, producing a
/// 128-character line that parses as a 64-byte key and is refused.
#[test]
fn three_keys_appended_one_at_a_time_with_no_trailing_newline_parse() {
    let text = "5108de6435c6a6e501f8905629c908f13ff8a348193d55172f4572b966131107\n\
                93f16e829493bd4b61fd56f9437dd90d28b96942199da2662c3be1afbe18905d\n\
                62bc792e1cfb85fe284c530220727164a415676dc2d7c6c9d0696f4a0fcbb233";
    assert_eq!(text.len(), 194, "the byte count a board reports for this file");

    let pins = parse_pins(text).expect("a pin file with no trailing newline");
    assert_eq!(pins.len(), 3, "three keys, and the last one is not lost");
    assert_eq!(pins[2][0], 0x62, "the appended key is whole");

    // And the failure it is guarding: the same three keys with the last two run
    // together, which is what appending without a leading newline produces.
    let run_together = text.replacen('\n', "", 1);
    let refusal = parse_pins(&run_together).expect_err("a 128-character line is not a key");
    assert!(refusal.contains("32 bytes"), "{refusal}");
}

/// An empty pin file parses to nothing, and it is [`server_config`] that
/// refuses it — one refusal, in the place that knows it matters.
#[test]
fn an_empty_pin_file_is_not_an_error_here_but_is_refused_where_it_is_used() {
    assert_eq!(parse_pins("\n# nobody yet\n\n"), Ok(Vec::new()));
    let id = mint("board", false);
    assert_eq!(server_config(&id, Vec::new(), provider()).err(), Some(Error::NoPins));
}

/// A malformed pin names its line, because a pin file is edited by hand.
#[test]
fn a_bad_pin_names_the_line_it_is_on() {
    let bad = parse_pins("aabb\nnot-hex-at-all\n").expect_err("refused");
    assert!(bad.contains("line 1"), "{bad}");
    assert!(bad.contains("32 bytes"), "{bad}");

    let odd = parse_pins("zz\n").expect_err("refused");
    assert!(odd.contains("hexadecimal"), "{odd}");
}

/// Hex round-trips, so a key printed by tooling is a key a pin file accepts.
#[test]
fn a_printed_key_is_one_a_pin_file_reads_back() {
    let id = mint("board", false);
    let key = key_of(&id);
    let printed = encode_hex(&key);
    assert_eq!(parse_pins(&printed).expect("read back"), vec![key]);
}

// --- the forgery the pin used to admit --------------------------------------
//
// `PLAN.md` §123.2 item 1. Until 2026-09-03 `public_key_in_certificate` took the
// first Ed25519-SPKI-shaped window anywhere in the DER, while `rustls` verified
// the handshake against the certificate's real one. Planting a pinned key
// earlier in the same certificate therefore satisfied both: this crate said
// "pinned" about key A while the peer proved possession of key B.
//
// These four build that certificate by hand rather than through `rcgen`,
// deliberately. `rcgen` will not put arbitrary bytes in a name — an Ed25519
// public key is not valid UTF-8 and not a PrintableString — so a test that went
// through it could only mint something an attacker would not. The attacker is
// not using `rcgen`.

/// A DER length, short form or long.
fn der_length(n: usize) -> Vec<u8> {
    if n < 0x80 {
        return vec![n as u8];
    }
    let bytes: Vec<u8> = n.to_be_bytes().into_iter().skip_while(|b| *b == 0).collect();
    let mut out = vec![0x80 | bytes.len() as u8];
    out.extend_from_slice(&bytes);
    out
}

/// `tag`, its length, and `content`.
fn der(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    out.extend_from_slice(&der_length(content.len()));
    out.extend_from_slice(content);
    out
}

/// The forty-four bytes of an Ed25519 `SubjectPublicKeyInfo` — what the old
/// implementation searched for and what an attacker plants.
fn spki_bytes(key: &Pin) -> Vec<u8> {
    let mut out = SPKI_ED25519_PREFIX.to_vec();
    out.extend_from_slice(key);
    out
}

/// **The attack, as a certificate.** Structurally a valid X.509 leaf whose real
/// `subjectPublicKeyInfo` holds `real`, with `planted`'s SPKI carried in the
/// `subject` — which precedes it, and which nothing here ever looks inside.
fn forged(real: &Pin, planted: &Pin) -> Vec<u8> {
    // The Ed25519 AlgorithmIdentifier: SEQUENCE { OID 1.3.101.112 }.
    let algorithm = der(0x30, &[0x06, 0x03, 0x2b, 0x65, 0x70]);

    let mut tbs = Vec::new();
    tbs.extend_from_slice(&der(0xa0, &der(0x02, &[0x02]))); // [0] version v3
    tbs.extend_from_slice(&der(0x02, &[0x01])); // serialNumber
    tbs.extend_from_slice(&algorithm); // signature
    tbs.extend_from_slice(&der(0x30, &[])); // issuer, empty
    tbs.extend_from_slice(&der(0x30, &[])); // validity, empty
    // subject — where the pinned key goes. A real attacker has a dozen places
    // to put it; one is enough to prove the shape.
    tbs.extend_from_slice(&der(0x30, &der(0x04, &spki_bytes(planted))));
    tbs.extend_from_slice(&spki_bytes(real)); // subjectPublicKeyInfo

    let mut certificate = der(0x30, &tbs);
    certificate.extend_from_slice(&algorithm); // signatureAlgorithm
    certificate.extend_from_slice(&der(0x03, &[0x00; 65])); // signatureValue
    der(0x30, &certificate)
}

/// **The test is only a regression test if the bait is really in the bait
/// position.** If a later edit to `forged` put the planted key after the real
/// one, every assertion below would still pass and would prove nothing.
#[test]
fn the_forged_certificate_really_does_carry_the_planted_key_first() {
    let real = [0xBBu8; KEY_BYTES];
    let planted = [0xAAu8; KEY_BYTES];
    let der = forged(&real, &planted);

    let find = |needle: &[u8]| der.windows(needle.len()).position(|w| w == needle);
    let at_planted = find(&spki_bytes(&planted)).expect("the planted SPKI is in there");
    let at_real = find(&spki_bytes(&real)).expect("so is the real one");
    assert!(
        at_planted < at_real,
        "the planted key must come FIRST or this test cannot detect the old search: \
         planted at {at_planted}, real at {at_real}"
    );
}

/// **The key that comes back is the one the handshake is signed with.**
///
/// This is the assertion the old implementation failed: it answered `planted`,
/// which is the pinned key, about a certificate `rustls` would verify against
/// `real`.
#[test]
fn a_planted_key_is_not_read_because_the_field_is_found_by_position() {
    let real = [0xBBu8; KEY_BYTES];
    let planted = [0xAAu8; KEY_BYTES];
    assert_eq!(public_key_in_certificate(&forged(&real, &planted)), Some(real));
}

/// **And therefore the pin refuses it.** Knowing a pinned public key — which is
/// public by design, `ADR-025` §2 — is no longer enough to present one.
#[test]
fn pinning_a_key_does_not_admit_a_certificate_that_merely_contains_it() {
    let pinned = [0xAAu8; KEY_BYTES];
    let attacker = [0xBBu8; KEY_BYTES];
    let peer = PinnedPeer::new(vec![pinned], provider()).expect("a verifier");
    let certificate = CertificateDer::from(forged(&attacker, &pinned));
    assert_eq!(
        peer.check(&certificate, 0),
        Err(Error::NotPinned),
        "a certificate carrying a pinned key in its subject is not that key's holder"
    );
}

/// **The implementation this replaced, kept here and nowhere else, being
/// fooled.**
///
/// Without this the three tests above prove only that the current code is
/// self-consistent — a reader cannot tell from them whether the change fixed
/// anything. This runs the old one-line search against the same certificate and
/// shows the two answers differ, so the fix is a measurement rather than a
/// claim. If it ever stops differing, the search has come back.
#[test]
fn the_search_this_replaced_would_have_been_fooled_by_the_same_bytes() {
    let real = [0xBBu8; KEY_BYTES];
    let planted = [0xAAu8; KEY_BYTES];
    let der = forged(&real, &planted);

    let by_search: Option<Pin> = der
        .windows(SPKI_ED25519_PREFIX.len() + KEY_BYTES)
        .find(|w| w.starts_with(&SPKI_ED25519_PREFIX))
        .and_then(|w| w[SPKI_ED25519_PREFIX.len()..].try_into().ok());

    assert_eq!(by_search, Some(planted), "the search reads the key the attacker planted");
    assert_eq!(public_key_in_certificate(&der), Some(real), "the walk reads the real one");
    assert_ne!(by_search, public_key_in_certificate(&der), "and they are not the same key");
}

/// The walk refuses malformed DER rather than reading past the end of it. Each
/// of these is a shape a hostile peer can send for free.
#[test]
fn a_malformed_certificate_is_refused_rather_than_half_read() {
    let good = forged(&[0xBBu8; KEY_BYTES], &[0xAAu8; KEY_BYTES]);

    // Truncated anywhere is None, at every length, and none of them panic.
    for n in 0..good.len() {
        assert_eq!(public_key_in_certificate(&good[..n]), None, "truncated to {n} bytes");
    }
    // The indefinite-length form, which BER allows and DER forbids.
    assert_eq!(public_key_in_certificate(&[0x30, 0x80, 0x00, 0x00]), None);
    // A length that claims more than the buffer holds.
    assert_eq!(public_key_in_certificate(&[0x30, 0x7f, 0x00]), None);
    // The high-tag-number form.
    assert_eq!(public_key_in_certificate(&[0x3f, 0x01, 0x00]), None);
    // A TBSCertificate that runs out before the public key.
    let short = der(0x30, &der(0x30, &der(0x02, &[0x01])));
    assert_eq!(public_key_in_certificate(&short), None);
}

/// A missing identity says what to run, not just what is absent — the machine
/// that hits this is headless and the message is all its operator gets.
#[test]
fn a_missing_identity_names_the_file_and_the_remedy() {
    let nowhere = std::path::Path::new("/definitely/not/here");
    let why = read_identity(nowhere).expect_err("no identity is there");
    assert!(why.contains(SEED_FILE), "{why}");
    assert!(why.contains("cargo rivet identity new"), "{why}");
    assert!(why.contains("ADR-025"), "{why}");
}

/// **TLS 1.2 stays out, and NO manifest anywhere can bring it back.**
///
/// `.rulings/` R-140. The trap is `cargo`'s feature unification: `rustls` is
/// one crate in the build graph, so `tls12` enabled by **any** consumer is
/// `tls12` compiled into **all** of them.
///
/// # This test was wrong first, and the board found it
///
/// The first version checked two manifests — this crate's and
/// `cmd/rivet-fleetd`'s — because those were the two in front of whoever wrote
/// it. It passed. A card was built, installed and booted, and `openssl
/// s_client -tls1_2` against `rivet-mcpd` on 7777 still reached `Protocol:
/// TLSv1.2`, because **`cmd/rivet-mcpd`, `cmd/rivet-proxyd` and `rivet-remote`
/// each declared their own `rustls` with `tls12`** and nothing had looked at
/// them. A guard scoped to the files its author happened to be editing is the
/// defect it was written to prevent, wearing the guard's clothes.
///
/// So it walks **every `Cargo.toml` in the repository**. `vendor/` is excluded
/// for the reason `tools/docs/claims.py` excludes it — foreign source is not
/// this project's to hold to this rule — and `target/` is build output.
///
/// The manifests are read rather than the runtime asked, because with the
/// feature off there is no 1.2 to negotiate and nothing to observe: the
/// evidence exists only in the build inputs. If `rustls` ever exposes the
/// enabled version set at runtime, that is the better assertion — and after
/// this, a runtime one measured on a board is better still.
#[test]
fn no_manifest_in_the_repository_enables_tls12() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("rivet-tls sits under the repository root")
        .to_path_buf();

    let mut checked = 0usize;
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                // Build output, git internals, agent worktrees, scratch
                // packages, and foreign source we do not hold to this rule.
                if matches!(name.as_ref(), "target" | "vendor" | "node_modules")
                    || name.starts_with('.')
                {
                    continue;
                }
                stack.push(path);
            } else if name == "Cargo.toml" {
                checked += 1;
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                // Dependency lines only — the prose above them explains the
                // ruling and necessarily says the word.
                for line in text.lines().filter(|l| {
                    let t = l.trim_start();
                    t.starts_with("rustls") || t.starts_with("tokio-rustls")
                }) {
                    assert!(
                        !line.contains("tls12"),
                        "{} enables tls12:\n    {}\nCargo unifies features, so this compiles \
                         TLS 1.2 into EVERY consumer of rustls in the tree, including \
                         rivet-tls — and a board will accept a 1.2 handshake no matter what \
                         this crate's own manifest says.",
                        path.display(),
                        line.trim()
                    );
                }
            }
        }
    }

    // A walk that found nothing would pass silently, which is the failure this
    // whole test exists to stop happening a second time.
    assert!(
        checked >= 20,
        "only {checked} manifests were walked; this repository has more than thirty, so the \
         walk is broken and a pass here means nothing"
    );
}

// **There is deliberately no test of `no_tls12` itself**, and the reason is
// worth stating rather than leaving as an omission.
//
// It cannot be called: `DigitallySignedStruct::new` is private to `rustls`, so
// nothing outside that crate can build the argument. And it cannot be reached
// in a handshake either, because with the `tls12` feature off there is no 1.2
// to negotiate — which is the point of the change.
//
// So the hook is unreachable code that exists to keep the trait implemented and
// to fail loudly if the feature ever returns. The thing that can actually go
// wrong is the feature coming back, and `neither_manifest_enables_tls12` above
// is the test for that.

// ---------------------------------------------------------------------------
// The subject's commonName — a LABEL, and never an authorisation
// ---------------------------------------------------------------------------

/// A leaf whose subject and issuer carry DIFFERENT common names, which is what
/// every real certificate does and what a search would get wrong.
fn named(subject: &str) -> Vec<u8> {
    let issuer_key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).expect("a pair");
    let mut issuer_params = rcgen::CertificateParams::new(Vec::new()).expect("params");
    issuer_params.distinguished_name = {
        let mut dn = rcgen::DistinguishedName::new();
        dn.push(rcgen::DnType::CommonName, "home-fleet");
        dn
    };
    issuer_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let issuer = rcgen::Issuer::new(issuer_params, issuer_key);

    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).expect("a pair");
    let mut params = rcgen::CertificateParams::new(Vec::new()).expect("params");
    params.distinguished_name = {
        let mut dn = rcgen::DistinguishedName::new();
        dn.push(rcgen::DnType::CommonName, subject);
        dn
    };
    params.is_ca = rcgen::IsCa::NoCa;
    params.signed_by(&key, &issuer).expect("a leaf").der().to_vec()
}

/// **The reason this walks rather than searches.** A certificate carries two
/// common names — the issuer's and the subject's — and the issuer comes FIRST.
/// A search would put the fleet authority's name where the caller's belongs.
#[test]
fn the_subject_is_read_and_not_the_issuer_that_precedes_it() {
    let der = named("operator");
    assert_eq!(subject_common_name(&der).as_deref(), Some("operator"));
}

/// The name a person actually sees on the index page.
#[test]
fn an_ordinary_operator_name_comes_back_whole() {
    for name in ["operator", "someone", "Your Name", "rivet-01.local"] {
        assert_eq!(
            subject_common_name(&named(name)).as_deref(),
            Some(name),
            "{name} did not survive"
        );
    }
}

/// A subject with no `commonName` is legal, and answers nothing rather than
/// something wrong.
#[test]
fn a_subject_with_no_common_name_is_absent_rather_than_guessed() {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).expect("a pair");
    let mut params = rcgen::CertificateParams::new(vec!["host.local".to_string()])
        .expect("params");
    params.distinguished_name = rcgen::DistinguishedName::new();
    params.is_ca = rcgen::IsCa::NoCa;
    let der = params.self_signed(&key).expect("a leaf").der().to_vec();
    assert_eq!(subject_common_name(&der), None);
}

/// Nothing that is not a certificate produces a name, and nothing panics.
#[test]
fn rubbish_answers_nothing_rather_than_panicking() {
    for bytes in [
        b"".as_slice(),
        b"not der".as_slice(),
        &[0x30],
        &[0x30, 0x82, 0xff, 0xff],
        &[0x30, 0x03, 0x30, 0x01, 0x00],
    ] {
        assert_eq!(subject_common_name(bytes), None, "{bytes:?} produced a name");
    }
}

/// **Truncation must not become a name.** Every prefix of a real certificate is
/// either refused or answers the same thing; none of them invents one.
#[test]
fn no_prefix_of_a_certificate_invents_a_different_name() {
    let der = named("operator");
    for cut in 1..der.len() {
        match subject_common_name(&der[..cut]) {
            None => {}
            Some(name) => assert_eq!(name, "operator", "a {cut}-byte prefix said {name:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// A pin set replaced while the server runs — `ADR-063` §3
// ---------------------------------------------------------------------------

/// A board serving with a pin set its caller keeps, and can replace.
fn live(board: &Identity, pins: Vec<Pin>) -> (Arc<PinnedPeer>, Arc<ServerConfig>) {
    let p = provider();
    let verifier = Arc::new(PinnedPeer::new(pins, p.clone()).expect("a verifier"));
    let config = live_server_config(board, Arc::clone(&verifier), p).expect("a live config");
    (verifier, Arc::new(config))
}

/// A client configuration for `operator` pinning `board`. **Kept between
/// connections by the tests that need it**, because its session cache is what
/// would offer a resumption.
fn operator_config(operator: &Identity, board: &Identity) -> Arc<ClientConfig> {
    Arc::new(client_config(operator, vec![key_of(board)], provider()).expect("a client config"))
}

/// One connection, handshaken, with anything the server sends after the
/// handshake — a session ticket, if it issues one — delivered to the client.
fn dial(
    server: &Arc<ServerConfig>,
    client: &Arc<ClientConfig>,
) -> Result<(rustls::ClientConnection, rustls::ServerConnection), rustls::Error> {
    let mut s = rustls::ServerConnection::new(Arc::clone(server)).expect("a server connection");
    let mut c = rustls::ClientConnection::new(
        Arc::clone(client),
        rustls::pki_types::ServerName::try_from("board").expect("a name"),
    )
    .expect("a client connection");
    handshake(&mut c, &mut s)?;
    if c.is_handshaking() || s.is_handshaking() {
        return Err(rustls::Error::General("the handshake never finished".to_string()));
    }
    let mut wire = Vec::new();
    s.write_tls(&mut wire).expect("an in-memory write");
    if !wire.is_empty() {
        c.read_tls(&mut wire.as_slice()).expect("an in-memory read");
        c.process_new_packets()?;
    }
    Ok((c, s))
}

/// `ping` from the client, read by the server — a session that still works.
fn carries_data(c: &mut rustls::ClientConnection, s: &mut rustls::ServerConnection) -> bool {
    c.writer().write_all(b"ping").expect("a write");
    let mut wire = Vec::new();
    c.write_tls(&mut wire).expect("a flush");
    s.read_tls(&mut wire.as_slice()).expect("a read");
    if s.process_new_packets().is_err() {
        return false;
    }
    let mut got = [0u8; 8];
    matches!(s.reader().read(&mut got), Ok(4)) && &got[..4] == b"ping"
}

/// **An addition governs the very next handshake.** Refused before, accepted
/// after, with nothing restarted.
#[test]
fn a_key_added_to_a_live_set_is_accepted_by_the_next_handshake() {
    let board = mint("board", false);
    let operator = mint("operator", false);
    let newcomer = mint("newcomer", false);
    let (verifier, server) = live(&board, vec![key_of(&operator)]);

    dial(&server, &operator_config(&newcomer, &board))
        .expect_err("a key nobody pinned completed a handshake");

    verifier.replace(vec![key_of(&operator), key_of(&newcomer)]).expect("a set of two");
    assert_eq!(verifier.pinned(), 2);
    dial(&server, &operator_config(&newcomer, &board))
        .expect("the added key was refused after it was added");
    dial(&server, &operator_config(&operator, &board)).expect("the key that was there all along");
}

/// **A removal refuses the next handshake and leaves a session already made
/// alone.** `ADR-063` §3: existing sessions are not torn down.
#[test]
fn a_key_removed_is_refused_next_time_and_its_open_session_keeps_working() {
    let board = mint("board", false);
    let operator = mint("operator", false);
    let leaving = mint("leaving", false);
    let (verifier, server) = live(&board, vec![key_of(&operator), key_of(&leaving)]);

    let (mut c, mut s) =
        dial(&server, &operator_config(&leaving, &board)).expect("pinned, so admitted");
    verifier.replace(vec![key_of(&operator)]).expect("a set of one");

    assert!(carries_data(&mut c, &mut s), "the session made before the removal was cut");
    let err = dial(&server, &operator_config(&leaving, &board))
        .expect_err("a removed key completed a new handshake");
    assert!(format!("{err}").contains("not one this machine was told to expect"), "{err}");
}

/// An empty replacement is refused and the set in force is the set that was.
#[test]
fn an_empty_replacement_is_refused_and_the_old_set_stays() {
    let peer = PinnedPeer::new(vec![[1u8; KEY_BYTES]], provider()).expect("a verifier");
    assert_eq!(peer.replace(Vec::new()), Err(Error::NoPins));
    assert_eq!(peer.pins(), vec![[1u8; KEY_BYTES]]);
}

/// **A removed key cannot come back by resuming a session it made before.**
///
/// The same client configuration dials twice, so its session cache offers
/// whatever the first connection left it. [`live_server_config`] keeps no
/// sessions and issues no tickets, so the second connection is a full
/// handshake and the verifier is asked.
#[test]
fn a_removed_key_cannot_come_back_by_resuming_a_session() {
    let board = mint("board", false);
    let operator = mint("operator", false);
    let leaving = mint("leaving", false);
    let (verifier, server) = live(&board, vec![key_of(&operator), key_of(&leaving)]);
    let config = operator_config(&leaving, &board);

    dial(&server, &config).expect("pinned, so admitted");
    verifier.replace(vec![key_of(&operator)]).expect("a set of one");
    dial(&server, &config).expect_err("a removed key was admitted on its second connection");
}

/// **The measurement behind [`live_server_config`]'s two lines**: the same
/// removal against a server with rustls's default session handling, where the
/// removed key IS admitted again, by resumption, without the verifier being
/// asked. If this ever stops passing, rustls has started re-verifying on
/// resumption and the two lines are belt without braces — harmless, but the
/// reason given for them has changed.
#[test]
fn with_resumption_on_the_removed_key_would_have_been_admitted() {
    let board = mint("board", false);
    let operator = mint("operator", false);
    let leaving = mint("leaving", false);
    let p = provider();
    let verifier = Arc::new(
        PinnedPeer::new(vec![key_of(&operator), key_of(&leaving)], p.clone()).expect("a verifier"),
    );
    let server =
        Arc::new(server_config_verified_by(&board, Arc::clone(&verifier), p).expect("a config"));
    let config = operator_config(&leaving, &board);

    dial(&server, &config).expect("pinned, so admitted");
    verifier.replace(vec![key_of(&operator)]).expect("a set of one");
    dial(&server, &config)
        .expect("resumption admitted nothing, so the reason for turning it off has changed");
}
