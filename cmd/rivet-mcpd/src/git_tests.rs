//! What the CGI front end does with what git hands it.
//!
//! The two that matter: `Status:` becomes a status line and is **not**
//! forwarded, and `PATH` is set — `PLAN.md` §85.2 measured that its absence
//! produces a valid, empty advertisement rather than an error, which is the
//! worst kind of failure a git server can have.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

/// The three routes `ADR-035` §4 names, and the shapes around them.
#[test]
fn the_routes_git_serves_are_recognised_and_nothing_else_is() {
    for target in [
        "/example-repo.git/info/refs?service=git-upload-pack",
        "/example-repo.git/git-upload-pack",
        "/example-repo.git/git-receive-pack",
        "/deep/name.git/info/refs",
    ] {
        assert!(is_git(target), "{target} should route to git");
    }
    for target in [
        "/payload/rivet-abc-update.rvt",
        "/",
        "/info/refsomething",
        "/example-repo.git/objects/info/packs",
    ] {
        assert!(!is_git(target), "{target} should not route to git");
    }
}

/// **A destroyed repository awaiting deletion is not served** (`ADR-017` §3):
/// it is renamed aside under the repository root and stays there until a helper
/// has deleted it, and git would otherwise answer for it under that name.
#[test]
fn a_repository_set_aside_for_deletion_is_not_routed_to_git() {
    let aside = rivet_core::service::RECLAIM_MARK;
    for target in [
        format!("/{aside}5-1/info/refs?service=git-upload-pack"),
        format!("/{aside}5-1/git-upload-pack"),
        format!("/deep/{aside}5-1/git-receive-pack"),
    ] {
        assert!(!is_git(&target), "{target} was routed to git");
    }
}

/// **The header becomes a status line and is not forwarded.** CGI's `Status:`
/// is an instruction to the gateway, not part of the response; sending it on
/// would put a header where a client expects one and a status it already has.
#[test]
fn a_status_header_sets_the_status_and_is_removed() {
    let head = "Status: 404 Not Found\r\nContent-Type: text/plain\r\n";
    let (code, reason, forwarded) = translate(head).expect("a readable head");
    assert_eq!((code, reason), (404, "Not Found"));
    assert!(!forwarded.contains("Status:"), "the header was forwarded: {forwarded:?}");
    assert!(forwarded.contains("Content-Type: text/plain"), "{forwarded:?}");
}

/// No `Status:` means 200, which is CGI's rule and what git relies on for
/// every successful advertisement.
#[test]
fn no_status_header_means_two_hundred() {
    let head = "Content-Type: application/x-git-upload-pack-advertisement\r\n";
    let (code, reason, forwarded) = translate(head).expect("a readable head");
    assert_eq!((code, reason), (200, "OK"));
    assert!(forwarded.contains("x-git-upload-pack-advertisement"));
}

/// A code this build has no phrase for is still sent. A reason phrase is
/// decoration and refusing a response over one would be the tail wagging.
#[test]
fn an_unknown_status_code_is_forwarded_rather_than_refused() {
    let (code, reason, _) = translate("Status: 451 Unavailable\r\n").expect("readable");
    assert_eq!((code, reason), (451, "Status"));
}

/// A `Status:` this build cannot read is refused rather than guessed at — the
/// alternative is sending `200` for something git meant as a failure.
#[test]
fn a_malformed_status_is_refused_rather_than_defaulted() {
    let refused = translate("Status: not-a-number\r\n").expect_err("must refuse");
    assert!(refused.contains("cannot read"), "{refused}");
}

/// **`PATH` is set, and this is the test §85.2 asked for.** Its absence makes
/// `git-http-backend` serve an empty repository and report success.
#[test]
fn the_environment_sets_path_to_where_git_actually_is() {
    let env = environment("GET", "/r.git/info/refs", "service=git-upload-pack", None, None, None, None);
    let path = env
        .iter()
        .find(|(k, _)| k == "PATH")
        .map(|(_, v)| v.clone())
        .expect("PATH must be set or git serves empty repositories (PLAN.md §85.2)");
    assert_eq!(path, "/system/bin");
    assert!(
        rivet_core::git::BINARY.starts_with(&path),
        "PATH must contain git itself: {path} vs {}",
        rivet_core::git::BINARY
    );
}

/// The rest of what git needs, and the one that would be a security question if
/// it were wrong.
#[test]
fn the_environment_names_the_repository_root_and_exports_every_repository() {
    let env = environment("GET", "/r.git/info/refs", "service=git-upload-pack", None, None, None, None);
    let get = |key: &str| env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
    assert_eq!(get("GIT_PROJECT_ROOT"), Some(rivet_core::git::REPOSITORY_ROOT));
    assert_eq!(get("GIT_HTTP_EXPORT_ALL"), Some("1"));
    assert_eq!(get("HOME"), Some("/nonexistent"));
    assert_eq!(get("REQUEST_METHOD"), Some("GET"));
    assert_eq!(get("QUERY_STRING"), Some("service=git-upload-pack"));
}

/// `CONTENT_TYPE` and `CONTENT_LENGTH` appear only when there is a body — a
/// `GET` carrying either would be describing something that is not there.
#[test]
fn a_get_carries_no_body_headers_and_a_post_does() {
    let get = environment("GET", "/r.git/info/refs", "", None, None, None, None);
    assert!(!get.iter().any(|(k, _)| k == "CONTENT_TYPE" || k == "CONTENT_LENGTH"));

    let post = environment(
        "POST",
        "/r.git/git-upload-pack",
        "",
        Some("application/x-git-upload-pack-request"),
        None,
        Some(42),
        None,
    );
    let get_of = |key: &str| post.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
    assert_eq!(get_of("CONTENT_TYPE"), Some("application/x-git-upload-pack-request"));
    assert_eq!(get_of("CONTENT_LENGTH"), Some("42"));
    assert!(!post.iter().any(|(k, _)| k == "HTTP_CONTENT_ENCODING"), "a plain body names no encoding");
}

/// **A gzipped body reaches the CGI as one.** `git` compresses a fetch's
/// negotiation once it passes 1 KiB; without `HTTP_CONTENT_ENCODING`
/// `git-http-backend` hands the compressed bytes to upload-pack, which reads
/// the gzip magic as a pkt-line length and the fetch dies. Measured on a QEMU
/// guest, 2026-09-14 (`tools/qemu/adr-035-git-proof.sh`).
#[test]
fn a_gzipped_body_is_named_to_the_cgi() {
    let env = environment(
        "POST",
        "/r.git/git-upload-pack",
        "",
        Some("application/x-git-upload-pack-request"),
        Some("gzip"),
        Some(1500),
        None,
    );
    let encoding = env.iter().find(|(k, _)| k == "HTTP_CONTENT_ENCODING").map(|(_, v)| v.as_str());
    assert_eq!(encoding, Some("gzip"));
}

/// Both separators, because a CGI program may use either and accepting one
/// works until it does not.
#[test]
fn the_header_block_ends_at_either_spelling_of_a_blank_line() {
    assert_eq!(find_blank_line(b"A: b\r\n\r\nbody"), Some((4, 4)));
    assert_eq!(find_blank_line(b"A: b\n\nbody"), Some((4, 2)));
    assert_eq!(find_blank_line(b"A: b\r\nno end"), None);
}

/// The path of the binary and the directory on `PATH` are the same fact, so a
/// move of one must move the other.
#[test]
fn the_parent_of_a_path_is_its_directory() {
    assert_eq!(parent_of("/system/bin/git"), "/system/bin");
    assert_eq!(parent_of("git"), "git");
}

/// **The head ends with a blank line**, and this test exists because it did not.
/// Without it a client reads the pack's first bytes as another header; `curl`
/// says "Header without colon" and the clone fails a long way from the cause
/// (`PLAN.md` §88.6).
#[test]
fn the_response_head_ends_with_an_empty_line() {
    let head = response_head(200, "OK", "Content-Type: application/x-git-upload-pack-result\r\n");
    assert!(head.ends_with("\r\n\r\n"), "no blank line terminates the head: {head:?}");
    let (fields, rest) = head.split_once("\r\n\r\n").expect("a head/body boundary");
    assert!(rest.is_empty(), "nothing may follow the boundary: {rest:?}");
    assert!(fields.starts_with("HTTP/1.1 200 OK\r\n"), "{fields:?}");
    // Every line that is not the status line carries a colon — the exact
    // property the client checks.
    for line in fields.split("\r\n").skip(1).filter(|l| !l.is_empty()) {
        assert!(line.contains(':'), "{line:?} is a header without a colon");
    }
    assert!(fields.contains("Connection: close"), "{fields:?}");
}

/// git's own `Content-Length` is forwarded and none is added — two of them is
/// a response a client is entitled to reject.
#[test]
fn no_second_content_length_is_invented() {
    let head = response_head(200, "OK", "Content-Length: 42\r\n");
    assert_eq!(head.matches("Content-Length").count(), 1, "{head:?}");
}

/// **`REMOTE_USER` is the pin, and without it a push is refused.**
///
/// `git-http-backend` serves `git-receive-pack` only for an authenticated
/// caller, and it decides that by whether the gateway set `REMOTE_USER`. A push
/// over the control plane came back `403` until this was set (`PLAN.md` §88.9).
/// The value is the same `pinned-<hex>` principal an install records — one
/// identity, one spelling (`ADR-011`).
#[test]
fn the_principal_becomes_remote_user_and_is_the_pin() {
    let pin = "pinned-5108de6435c6a6e501f8905629c908f13ff8a348193d55172f4572b966131107";
    let env = environment("POST", "/r.git/git-receive-pack", "", None, None, Some(9), Some(pin));
    let user = env.iter().find(|(k, _)| k == "REMOTE_USER").map(|(_, v)| v.as_str());
    assert_eq!(user, Some(pin), "a push needs REMOTE_USER or git answers 403");
}

/// **And it is absent when nothing was pinned.** A caller over the plain
/// loopback transport presented no certificate, so it is not authenticated and
/// git refusing its push is the correct answer rather than an oversight.
#[test]
fn no_pin_means_no_remote_user() {
    let env = environment("POST", "/r.git/git-receive-pack", "", None, None, Some(9), None);
    assert!(
        !env.iter().any(|(k, _)| k == "REMOTE_USER"),
        "an unauthenticated caller was named as a user: {env:?}"
    );
}
