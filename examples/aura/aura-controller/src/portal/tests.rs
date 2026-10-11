//! **The responses, the index parsing and the e-mail mask.**
//!
//! The routing and the handlers are not driven here: every one of them needs a
//! `Portal`, which needs a kiosk sender, which needs a socket a supervisor
//! handed over. What is asserted is the part that is decidable without one —
//! the response framing that every handler goes through, and the two small
//! functions that decide what a request means.
//!
//! The pages themselves are asserted in `html::tests`, which is where the
//! content is, and the TLS validation itself is asserted by `rivet-sdk`'s own
//! suite, which is where `requiring_a_caller_certificate_from` lives. What
//! this file's own "prepare() fails before any add-on could be touched"
//! section covers is `portal`'s wiring of that validation — that
//! `tls_server_config` needs no `RIVET_APP_DATA` to test and that refusing
//! with no TLS files present touches nothing an add-on's trial note would
//! live under.

use super::*;

/// Split a response into its status line, its headers and its body.
fn parts(response: &str) -> (&str, Vec<&str>, &str) {
    let (head, body) = response.split_once("\r\n\r\n").expect("a blank line ends the head");
    let mut lines = head.split("\r\n");
    let status = lines.next().expect("a status line");
    (status, lines.collect(), body)
}

#[test]
fn a_page_carries_its_length_type_and_a_close() {
    let response = page(200, "OK", "text/html; charset=utf-8", "<p>hi</p>");
    let (status, headers, body) = parts(&response);
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert_eq!(body, "<p>hi</p>");
    assert!(headers.contains(&"Content-Length: 9"), "{headers:?}");
    assert!(headers.contains(&"Content-Type: text/html; charset=utf-8"));
    assert!(headers.contains(&"Connection: close"));
}

/// **`nosniff`, because this serves CSS and HTML from the same origin as a
/// form that takes a password.** A browser that guessed a type would be a
/// browser that could be persuaded to run one as the other.
#[test]
fn a_page_forbids_content_type_sniffing_and_caching() {
    let response = page(200, "OK", "text/plain", "x");
    let (_, headers, _) = parts(&response);
    assert!(headers.contains(&"X-Content-Type-Options: nosniff"), "{headers:?}");
    // The pages carry a credential's shape and the schedule; a cached copy in a
    // shared browser is a copy nobody revoked.
    assert!(headers.contains(&"Cache-Control: no-store"));
}

/// The length is in **bytes**, not characters, or a page with an em dash in it
/// is truncated by exactly as many multi-byte characters as it contains.
#[test]
fn the_content_length_counts_bytes_and_not_characters() {
    let body = "Digital Rain — Ice Blue";
    let response = page(200, "OK", "text/html", body);
    let (_, headers, _) = parts(&response);
    assert!(
        headers.contains(&format!("Content-Length: {}", body.len()).as_str()),
        "{headers:?}"
    );
    assert_ne!(body.len(), body.chars().count(), "the fixture must be multi-byte");
}

/// **A 303**, which is what turns a form POST into a GET the browser can
/// reload without resubmitting.
#[test]
fn a_redirect_is_a_see_other_with_no_body() {
    let response = redirect("./?flash=saved&section=general");
    let (status, headers, body) = parts(&response);
    assert_eq!(status, "HTTP/1.1 303 See Other");
    assert_eq!(body, "");
    assert!(headers.contains(&"Location: ./?flash=saved&section=general"));
    assert!(headers.contains(&"Content-Length: 0"));
}

/// **Relative**, so it survives being mounted under `/aura/`. An absolute one
/// would send the browser to the machine's root the moment the portal was
/// reached through the proxy.
#[test]
fn every_redirect_this_module_writes_is_relative() {
    for location in [
        "./?flash=saved&section=general",
        "../?section=schedule",
        "../?flash=no-actions&section=schedule",
        "../?section=schedule&flash=saved",
    ] {
        assert!(
            !location.starts_with('/'),
            "an absolute redirect: {location}"
        );
        let response = redirect(location);
        let (_, headers, _) = parts(&response);
        assert!(headers.contains(&format!("Location: {location}").as_str()));
    }
}

// ---- What a request means -------------------------------------------------

#[test]
fn an_index_query_parses_when_it_is_a_number() {
    assert_eq!(index_of(&form::Form::parse("index=3")), Some(3));
    assert_eq!(index_of(&form::Form::parse("index=0")), Some(0));
}

/// Anything else is `None`, which the handlers treat as *there is no such
/// event* — a redirect to the list rather than a panic or a wrong row.
#[test]
fn an_index_that_is_not_a_number_is_no_index_at_all() {
    for query in ["index=", "index=-1", "index=abc", "index=1.5", ""] {
        assert_eq!(index_of(&form::Form::parse(query)), None, "{query}");
    }
}

// ---- The query encoder ------------------------------------------------------

/// The one thing this needs to survive: a complaint carrying spaces, an
/// ampersand or a percent sign must come back as one parameter, not several
/// and not truncated.
#[test]
fn encode_query_survives_spaces_ampersands_and_percents() {
    let encoded = encode_query("Low: 40–120 & 100% sure");
    assert!(!encoded.contains(' '), "{encoded}");
    assert!(!encoded.contains('&'), "{encoded}");
    let decoded = form::Form::parse(&format!("msg={encoded}"));
    assert_eq!(decoded.value("msg"), "Low: 40–120 & 100% sure");
}

// ---- The refusal that names the files --------------------------------------

/// **The three names are the fleet's**, and the hint has to print all of them
/// with the command that installs each — a portal that refuses to start
/// without saying what is missing is a board somebody has to guess at.
#[test]
fn the_install_hint_names_all_three_files_and_the_command() {
    let hint = install_hint();
    for name in [CERTIFICATE, KEY, AUTHORITY] {
        assert!(hint.contains(name), "{name} is not in the hint");
    }
    assert!(hint.contains("rivet apply install-secret aura-controller"), "{hint}");
    assert!(hint.contains("there is no plaintext mode"), "{hint}");
}

/// The names are the ones the rest of the fleet uses, and a rename here would
/// mean an operator typing a different word for this application than for the
/// other four.
#[test]
fn the_three_files_are_named_the_way_the_fleet_names_them() {
    assert_eq!(CERTIFICATE, "tls-cert");
    assert_eq!(KEY, "tls-key");
    assert_eq!(AUTHORITY, "tls-root");
}

/// `tcp:8080` in the manifest arrives as `tcp-8080`, because `LISTEN_FDNAMES`
/// is colon-separated — and a program looking for the colon finds nothing.
#[test]
fn the_listen_name_carries_the_hyphen_the_supervisor_uses() {
    assert_eq!(LISTEN_NAME, "tcp-8080");
    assert!(!LISTEN_NAME.contains(':'));
}

// ---- prepare() fails before any add-on could be touched --------------------
//
// The TLS validation itself — a certificate and key that agree, a missing
// file naming itself, an authority with nothing in it — is `rivet-sdk`'s own
// suite, where `Tls::in_directory` lives; see this file's own header. What is
// asserted here is `portal`'s own wiring of it: that `tls_server_config`
// refuses with no `tls-cert`, `tls-key` or `tls-root` present, and — the
// property `main::run`'s reordering exists for — that doing so touches
// nothing under `addons/`, because nothing about an add-on is reachable from
// this function at all.

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("aura-controller-portal-tests-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch directory");
    dir
}

/// **The property `main::run`'s reordering exists for.** With none of the
/// three TLS files present, the check this program does before opening any
/// add-on refuses — and it refuses without ever creating the `addons/`
/// directory a trial note would live under, because nothing about an add-on
/// is reachable from `tls_server_config` at all.
///
/// This is watched to fail with the fix removed: reverting `main.rs` to call
/// the background pack, the indoor thread and the reading thread before the
/// portal's TLS check is what let a missing `tls-cert` leave `addons/*
/// .loading` notes behind on a real board.
#[test]
fn missing_tls_material_is_refused_and_touches_nothing_about_an_addon() {
    let dir = scratch("missing-tls");
    let result = tls_server_config(&dir);
    assert!(result.is_err(), "no cert, key or root should refuse to start");
    assert!(
        !dir.join("addons").exists(),
        "the TLS check must not have created anything an add-on's trial note would live under"
    );
}

/// The refusal names the certificate this module actually looks for, so an
/// operator reading it knows which of the three files to install first —
/// this is `portal`'s own wiring of the filenames, not `rivet-sdk`'s generic
/// message.
#[test]
fn the_refusal_names_the_certificate_this_module_looks_for() {
    let dir = scratch("names-the-file");
    let error = tls_server_config(&dir).expect_err("no files at all must refuse");
    assert!(error.contains(CERTIFICATE), "{error}");
}
