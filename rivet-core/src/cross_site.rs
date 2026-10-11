//! **Whether a request that changes something was caused by a page on another
//! site** — `.rulings/` R-186 D1, `ADR-046` §4, `ADR-043` §5.
//!
//! # The attack this answers
//!
//! A browser holding the operator's client certificate presents it to **any**
//! request to a machine, whichever page caused the request. A page on another
//! site can make that browser `POST` a body of one of the three CORS-safelisted
//! types — `text/plain`, `application/x-www-form-urlencoded`,
//! `multipart/form-data` — with no preflight, so the request arrives
//! authenticated as the operator. Behind `rivet-proxyd` that is an
//! application's restore route replacing its data, or any other write; on
//! `rivet-fleetd` it is `rivet_apply` against a machine.
//!
//! # The two headers, and why each is trusted only as far as it goes
//!
//! **`Sec-Fetch-Site` is the answer when it is present.** Page script cannot
//! set it (it is a forbidden request header), so a value a browser sent is the
//! browser's own account of who caused the request. Only `same-origin` (a page
//! on this origin) and `none` (the operator typed the address, or used a
//! bookmark) pass; `same-site` and `cross-site` are refused, as is any value
//! this build does not define.
//!
//! **`Origin` is the answer when `Sec-Fetch-Site` is absent.** Page script
//! cannot set that either. It must equal this request's own origin — the scheme
//! the server terminated and the `Host` the request named — and `null`, a value
//! that is not an origin, or an `Origin` with no single `Host` to compare it to
//! is cross-site.
//!
//! **A request carrying neither passes.** That is `curl`, `cargo rivet backup`
//! and `restore`, an application reaching another, and every other client that
//! is not a browser; requiring a browser header of them would make an
//! operator's own tools impersonate a browser. The assumption this rests on is
//! stated rather than hidden: **every browser engine that attaches a client
//! certificate to a cross-site request sends at least one of the two headers on
//! an unsafe one.** Chromium, Gecko and WebKit send `Origin` on every
//! cross-origin `POST`, and `Sec-Fetch-Site` on every request in current
//! releases. A browser that sent neither would not be stopped here.
//!
//! # Only methods that change something are refused
//!
//! `GET`, `HEAD` and `OPTIONS` pass whatever the headers say. A cross-site
//! `GET` is a navigation or an image load, and a site links to another as a
//! matter of course; a `GET` that changes something is the application's defect
//! (`ADR-051` §12 has the restore route take `POST`). `OPTIONS` passes so a
//! CORS preflight reaches whoever answers it — and a preflight that some
//! application answered too generously still cannot carry the write that
//! follows, because that write carries `Sec-Fetch-Site: cross-site` and is
//! refused here. Methods are case-sensitive (RFC 9110 §9.1), so `get` is not
//! `GET` and is refused like any other method this does not name.
//!
//! # Why it lives in `rivet-core`
//!
//! Two programs ask it — `rivet-proxyd` for every application behind it and
//! `rivet-fleetd` for its own administrative endpoints — and two copies of a
//! security rule drift (`ADR-011`). It reads a request head that is already a
//! string and performs no I/O, which is `ADR-014` §3's condition for being
//! here.

/// The methods that pass whatever a request's headers say.
pub const SAFE_METHODS: [&str; 3] = ["GET", "HEAD", "OPTIONS"];

/// The `Sec-Fetch-Site` values that pass.
///
/// `same-origin` is a page on this origin. `none` is a request no page caused —
/// an address typed or a bookmark followed — which a page on another site
/// cannot produce.
pub const PASSING_FETCH_SITES: [&str; 2] = ["same-origin", "none"];

/// Whether `method` is one [`refuses`] never refuses.
#[must_use]
pub fn is_safe_method(method: &str) -> bool {
    SAFE_METHODS.contains(&method)
}

/// Every value of one header in a request head, in order, case-insensitively by
/// name (RFC 9110 §5.1), each trimmed.
///
/// The first line is the request line and is skipped. A line with no colon is
/// not a header and is skipped.
pub fn header_values<'a>(head: &'a str, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    head.lines().skip(1).filter_map(move |line| {
        let (key, value) = line.split_once(':')?;
        key.trim().eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

/// One header's first value, case-insensitively by name.
///
/// Browsers do not agree on the case they send, so a comparison that did not
/// fold case would be a security check that worked in one browser.
#[must_use]
pub fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().skip(1).find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim().eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

/// The method on a request head's first line, or `""` for a head with none.
#[must_use]
pub fn method(head: &str) -> &str {
    head.lines()
        .next()
        .and_then(|line| line.split(' ').next())
        .unwrap_or("")
}

/// Whether the browser that sent this request says a page on another site
/// caused it. `scheme` is the one the server terminated — `"https"` for every
/// Rivet listener that asks.
///
/// See the module documentation for the rule and the assumption it rests on.
/// **Every value of each header is checked**, so a head carrying a passing
/// `Sec-Fetch-Site` beside a failing one is cross-site rather than whichever
/// came first.
#[must_use]
pub fn is_cross_site(head: &str, scheme: &str) -> bool {
    let mut fetch_sites = header_values(head, "sec-fetch-site").peekable();
    if fetch_sites.peek().is_some() {
        return fetch_sites.any(|value| {
            !PASSING_FETCH_SITES
                .iter()
                .any(|passing| value.eq_ignore_ascii_case(passing))
        });
    }

    let mut origins = header_values(head, "origin").peekable();
    if origins.peek().is_none() {
        return false;
    }
    // **Exactly one `Host`**, or there is no origin of our own to compare with.
    // RFC 9112 §3.2 has a server refuse a request with more than one.
    let mut hosts = header_values(head, "host");
    let (Some(host), None) = (hosts.next(), hosts.next()) else {
        return true;
    };
    let own = origin_of(scheme, host);
    origins.any(|origin| normalised(scheme, origin) != own)
}

/// Whether a request must be refused before anything acts on it: a method that
/// changes something, caused by a page on another site.
#[must_use]
pub fn refuses(head: &str, scheme: &str) -> bool {
    !is_safe_method(method(head)) && is_cross_site(head, scheme)
}

/// `scheme://host`, lower-cased, with the scheme's default port removed.
fn origin_of(scheme: &str, host: &str) -> String {
    let host = host.trim().to_ascii_lowercase();
    let host = strip_default_port(scheme, &host);
    format!("{}://{host}", scheme.to_ascii_lowercase())
}

/// An `Origin` value, lower-cased, with its scheme's default port removed.
///
/// Anything that is not `<scheme>://<host>` — `null`, a path, a different
/// scheme — comes back as something [`origin_of`] never produces, so it cannot
/// compare equal.
fn normalised(scheme: &str, origin: &str) -> String {
    let origin = origin.trim().to_ascii_lowercase();
    let prefix = format!("{}://", scheme.to_ascii_lowercase());
    match origin.strip_prefix(&prefix) {
        Some(host) if !host.is_empty() && !host.contains('/') => {
            format!("{prefix}{}", strip_default_port(scheme, host))
        }
        _ => format!("not an origin of this scheme: {origin}"),
    }
}

/// `host:443` is `host` for `https`, and `host:80` is `host` for `http` — the
/// spelling a browser uses in `Origin` and `Host` alike.
fn strip_default_port<'a>(scheme: &str, host: &'a str) -> &'a str {
    let default = if scheme.eq_ignore_ascii_case("https") {
        ":443"
    } else if scheme.eq_ignore_ascii_case("http") {
        ":80"
    } else {
        return host;
    };
    host.strip_suffix(default).unwrap_or(host)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    const HOST: &str = "rivet-01.local";
    const OWN: &str = "https://rivet-01.local";

    /// What `Sec-Fetch-Site` says, if anything.
    #[derive(Clone, Copy, Debug)]
    enum Fetch {
        Absent,
        Is(&'static str),
    }

    /// What `Origin` says, if anything.
    #[derive(Clone, Copy, Debug)]
    enum Org {
        Absent,
        Same,
        Other,
        Null,
    }

    fn head(method: &str, fetch: Fetch, origin: Org) -> String {
        let mut head =
            format!("{method} /tempmon/api/v1/restore?confirm=true HTTP/1.1\r\nHost: {HOST}\r\n");
        if let Fetch::Is(value) = fetch {
            head.push_str(&format!("Sec-Fetch-Site: {value}\r\n"));
        }
        match origin {
            Org::Absent => {}
            Org::Same => head.push_str(&format!("Origin: {OWN}\r\n")),
            Org::Other => head.push_str("Origin: https://elsewhere.example\r\n"),
            Org::Null => head.push_str("Origin: null\r\n"),
        }
        head.push_str("Content-Type: text/plain\r\nContent-Length: 2\r\n\r\n");
        head
    }

    /// **The decision table, whole.** Every method × every `Sec-Fetch-Site`
    /// value × every `Origin`, against one statement of the rule written out
    /// separately from the code under test.
    #[test]
    fn the_decision_table() {
        let methods = [
            "GET", "HEAD", "OPTIONS", "POST", "PUT", "PATCH", "DELETE", "get", "BREW",
        ];
        let fetches = [
            Fetch::Absent,
            Fetch::Is("same-origin"),
            Fetch::Is("none"),
            Fetch::Is("same-site"),
            Fetch::Is("cross-site"),
            Fetch::Is("Cross-Site"),
            Fetch::Is("something-new"),
            Fetch::Is(""),
        ];
        let origins = [Org::Absent, Org::Same, Org::Other, Org::Null];

        let mut refused = 0;
        let mut rows = 0;
        for method in methods {
            for fetch in fetches {
                for origin in origins {
                    let safe = matches!(method, "GET" | "HEAD" | "OPTIONS");
                    let cross = match fetch {
                        Fetch::Is(v) => {
                            !matches!(v.to_ascii_lowercase().as_str(), "same-origin" | "none")
                        }
                        Fetch::Absent => matches!(origin, Org::Other | Org::Null),
                    };
                    let expected = !safe && cross;
                    let text = head(method, fetch, origin);
                    assert_eq!(
                        refuses(&text, "https"),
                        expected,
                        "{method} with Sec-Fetch-Site {fetch:?} and Origin {origin:?}"
                    );
                    assert_eq!(is_cross_site(&text, "https"), cross, "{fetch:?} {origin:?}");
                    rows += 1;
                    refused += usize::from(expected);
                }
            }
        }
        assert_eq!(rows, 9 * 8 * 4);
        // Six unsafe methods × (five refusing `Sec-Fetch-Site` rows × four
        // origins + the absent row's two refusing origins).
        assert_eq!(refused, 6 * (5 * 4 + 2));
    }

    /// **The attack, spelled the three ways a page on another site can spell
    /// it**, each refused — and by `Origin` alone, for an engine that sent no
    /// `Sec-Fetch-Site`.
    #[test]
    fn a_cross_site_post_of_each_safelisted_type_is_refused() {
        for content_type in [
            "text/plain",
            "application/x-www-form-urlencoded",
            "multipart/form-data; boundary=x",
        ] {
            let by_fetch = format!(
                "POST /reliquary/restore HTTP/1.1\r\nHost: {HOST}\r\nSec-Fetch-Site: cross-site\r\n\
                 Content-Type: {content_type}\r\n\r\n"
            );
            let by_origin = format!(
                "POST /reliquary/restore HTTP/1.1\r\nHost: {HOST}\r\nOrigin: https://evil.example\r\n\
                 Content-Type: {content_type}\r\n\r\n"
            );
            assert!(refuses(&by_fetch, "https"), "{by_fetch}");
            assert!(refuses(&by_origin, "https"), "{by_origin}");
        }
    }

    /// **The clients that must keep working.** `cargo rivet restore` sends
    /// neither header; an application's own page sends `same-origin` and its
    /// own `Origin`.
    #[test]
    fn the_legitimate_clients_pass() {
        let cargo_rivet = "POST /probe/api/v1/restore?confirm=true HTTP/1.1\r\nHost: rivet-01.local:443\r\n\
                           User-Agent: ureq/3\r\nContent-Type: application/vnd.sqlite3\r\nContent-Length: 2048\r\n\r\n";
        assert!(!refuses(cargo_rivet, "https"));

        let tempmon_page = "POST /tempmon/api/v1/restore?confirm=true HTTP/1.1\r\nHost: rivet-01.local\r\n\
                            Origin: https://rivet-01.local\r\nSec-Fetch-Site: same-origin\r\n\
                            Content-Type: application/vnd.sqlite3\r\n\r\n";
        assert!(!refuses(tempmon_page, "https"));

        // An engine that sends `Origin` and no `Sec-Fetch-Site`, on a port.
        let older_engine = "PUT /notes/api/notes/7 HTTP/1.1\r\nHost: restore-guest.local:8443\r\n\
                            Origin: https://restore-guest.local:8443\r\n\r\n";
        assert!(!refuses(older_engine, "https"));
    }

    /// Header names and the values that are words are matched without regard
    /// to case, because browsers disagree about the case they send.
    #[test]
    fn case_is_folded_where_http_folds_it() {
        let head =
            "POST /x HTTP/1.1\r\nHOST: Rivet-01.Local\r\nsec-fetch-site: Same-Origin\r\n\r\n";
        assert!(!refuses(head, "https"));
        let head =
            "POST /x HTTP/1.1\r\nhost: Rivet-01.Local\r\nORIGIN: HTTPS://RIVET-01.LOCAL\r\n\r\n";
        assert!(!refuses(head, "https"));
    }

    /// The default port is the same origin written two ways, on either side.
    #[test]
    fn the_default_port_is_the_same_origin() {
        let head = "POST /x HTTP/1.1\r\nHost: rivet-01.local:443\r\nOrigin: https://rivet-01.local\r\n\r\n";
        assert!(!refuses(head, "https"));
        let head = "POST /x HTTP/1.1\r\nHost: rivet-01.local\r\nOrigin: https://rivet-01.local:443\r\n\r\n";
        assert!(!refuses(head, "https"));
    }

    /// **Different port, different scheme, a neighbouring machine**: each a
    /// different origin, each refused.
    #[test]
    fn a_neighbouring_origin_is_not_this_one() {
        for origin in [
            "https://rivet-01.local:4443",
            "http://rivet-01.local",
            "https://rivet-02.local",
            "https://rivet-01.local.evil.example",
            "https://rivet-01.local/",
            "https://",
            "rivet-01.local",
        ] {
            let head =
                format!("POST /x HTTP/1.1\r\nHost: rivet-01.local\r\nOrigin: {origin}\r\n\r\n");
            assert!(
                refuses(&head, "https"),
                "{origin} was taken for this origin"
            );
        }
    }

    /// **Every value counts, not the first.** A passing header first and a
    /// failing one second is cross-site; so is an `Origin` with no `Host`, or
    /// with two.
    #[test]
    fn a_repeated_header_is_judged_on_every_value() {
        let head = "POST /x HTTP/1.1\r\nHost: rivet-01.local\r\nSec-Fetch-Site: same-origin\r\n\
                    Sec-Fetch-Site: cross-site\r\n\r\n";
        assert!(refuses(head, "https"));
        let head = "POST /x HTTP/1.1\r\nHost: rivet-01.local\r\nOrigin: https://rivet-01.local\r\n\
                    Origin: https://evil.example\r\n\r\n";
        assert!(refuses(head, "https"));
        let head = "POST /x HTTP/1.1\r\nOrigin: https://rivet-01.local\r\n\r\n";
        assert!(
            refuses(head, "https"),
            "an Origin with no Host has nothing to equal"
        );
        let head = "POST /x HTTP/1.1\r\nHost: rivet-01.local\r\nHost: evil.example\r\n\
                    Origin: https://rivet-01.local\r\n\r\n";
        assert!(refuses(head, "https"), "two Hosts name no single origin");
    }

    /// `Sec-Fetch-Site` is the answer when it is present: a `same-origin`
    /// report is not overturned by an `Origin` this rule would otherwise
    /// refuse, and a `cross-site` one is not rescued by a matching `Origin`.
    #[test]
    fn sec_fetch_site_decides_when_present() {
        assert!(!refuses(
            &head("POST", Fetch::Is("same-origin"), Org::Other),
            "https"
        ));
        assert!(refuses(
            &head("POST", Fetch::Is("cross-site"), Org::Same),
            "https"
        ));
    }

    #[test]
    fn a_head_with_no_request_line_has_no_safe_method() {
        assert_eq!(method(""), "");
        assert!(!is_safe_method(method("")));
        assert!(
            !refuses("", "https"),
            "and no browser header, so nothing to refuse"
        );
    }
}
