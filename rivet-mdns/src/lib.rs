//! **Resolving a machine's `<host>:<port>` to socket addresses — mDNS for
//! `.local`, the OS resolver for everything else.**
//!
//! The query counterpart to `cmd/rivet-mdnsd`, which is the *responder*:
//! that program announces `<hostname>.local`, and this one asks. Both are the
//! same wire format (`simple-mdns`) so an answer and a question cannot drift.
//!
//! # Why this is its own crate rather than a function in each caller
//!
//! Two control-plane clients dial machines and neither is the other:
//! `rivet-remote`'s `tls.rs` (the host tools — `cargo rivet run`/`deploy`/
//! `install`, `rivet-fleet`) and `cmd/rivet-fleetd`'s `node.rs` (a fleet
//! server, running on a card, reaching its members). Each did its own
//! [`std::net::ToSocketAddrs`] lookup, so each reported a DHCP member — one
//! whose register record carries no address, only a `<name>.local` derived by
//! `rivet_register::Device::dial_host` — as *unreachable*. Putting the `.local`
//! lookup in one crate both link is `ADR-011`: one fact, one owner, the same
//! reasoning `rivet-tls` records for the pin logic the card and the host share.
//!
//! # What it does NOT do
//!
//! It is not a resolver daemon and holds no cache: a call sends one query and
//! takes the first answer, then drops its socket. A name that is not `.local`
//! is handed to [`std::net::ToSocketAddrs`] exactly as before — this crate
//! changes what happens for `.local` and nothing else, so a static address, an
//! IP literal, or a real DNS name resolves precisely as it did.

use std::net::{SocketAddr, ToSocketAddrs as _};
use std::time::Duration;

use simple_mdns::sync_discovery::OneShotMdnsResolver;

/// The zone an mDNS name lives in — RFC 6762, the same suffix
/// `rivet_mdnsd::announce` attaches when it publishes.
const LOCAL_SUFFIX: &str = ".local";

/// How long the mDNS query itself waits for a `.local` name to answer before
/// giving up.
///
/// The responder's socket already times its own reads out at 100ms and this is
/// the deadline the one-shot resolver counts against once that query is sent —
/// chosen short because the caller is a person or a fleet cycle waiting on a
/// dial, not a background probe. It bounds only the mDNS phase: [`resolve`]
/// tries the OS resolver first, and that lookup's own wait is not counted
/// against this deadline.
pub const QUERY_TIMEOUT: Duration = Duration::from_secs(3);

/// **Resolve `host:port` to the socket addresses to try.**
///
/// A `<name>.local:<port>` is looked up over mDNS and its answer returned as a
/// single address; anything else is handed to the operating system's resolver
/// unchanged, which is what both callers did for every name before this crate.
///
/// The OS resolver runs first, for every name, `.local` included — see the
/// private `resolve_with` this delegates to. Only when it cannot place a
/// `.local` name does the mDNS query run, so the full wait before that name is
/// reported unreachable is the OS lookup's own time plus [`QUERY_TIMEOUT`], not
/// `QUERY_TIMEOUT` alone.
///
/// # Errors
///
/// For a `.local` name: that nothing answered within [`QUERY_TIMEOUT`], or that
/// the query itself could not be made. For any other name: whatever
/// [`std::net::ToSocketAddrs`] refuses, asked once — a refusal is reported, not
/// retried. Every message ends "Nothing has been sent." because resolution
/// precedes the dial: a caller that failed here has sent nothing.
pub fn resolve(host_port: &str) -> Result<Vec<SocketAddr>, String> {
    resolve_with(host_port, |h| {
        h.to_socket_addrs().map(|found| found.collect::<Vec<SocketAddr>>())
    })
}

/// [`resolve`], with the OS lookup taken as `os_resolve` rather than calling
/// [`std::net::ToSocketAddrs`] directly — the seam a test uses to count how
/// many times the lookup runs and to control what it returns, without opening
/// a real socket. Production code reaches this only through [`resolve`],
/// which always passes the real OS resolver.
fn resolve_with(
    host_port: &str,
    os_resolve: impl Fn(&str) -> std::io::Result<Vec<SocketAddr>>,
) -> Result<Vec<SocketAddr>, String> {
    // **The OS resolver first.** It answers IP literals, real DNS, and `.local`
    // on any platform whose libc resolves mDNS — Windows (Bonjour) and macOS do,
    // a Rivet board's musl does not. Trying it first means this crate changes
    // nothing for a host the OS can already place: it only supplies an answer
    // where the OS has none, so `cargo rivet run <host>.local` from a
    // workstation keeps resolving exactly as it did before this crate existed.
    //
    // Called exactly once. Its result — success or failure — is kept in
    // `os_lookup` and reused below rather than asking the OS the same question
    // twice.
    let os_lookup = os_resolve(host_port);
    if let Ok(addresses) = &os_lookup
        && !addresses.is_empty()
    {
        return Ok(addresses.clone());
    }
    // The OS could not place it. A `.local` name is answered by mDNS — the case
    // on a Rivet board reaching a DHCP member by name, where nothing else can.
    if let Some((name, port)) = local_target(host_port) {
        return query_local(name, port);
    }
    // Not `.local`: report the OS lookup's own refusal from above, not a fresh
    // one.
    os_lookup.map_err(|e| format!("cannot resolve {host_port}: {e}. Nothing has been sent."))
}

/// If `host_port` is a well-formed `<name>.local:<port>`, its name (without any
/// trailing dot) and port; otherwise `None`, which routes it to the OS
/// resolver.
///
/// Deliberately conservative: a malformed port, a missing port, or a bracketed
/// IPv6 literal all return `None` and fall through to [`std::net::ToSocketAddrs`],
/// so this never turns a name the OS could have resolved into an mDNS query it
/// cannot.
///
/// **The `.local` suffix is matched ASCII case-insensitively** — RFC 6762
/// §16 makes multicast DNS name comparison ASCII case-insensitive, the same
/// rule ordinary DNS names already follow (RFC 4343 §3) — and the name is
/// returned exactly as written, never case-folded: the comparison is
/// case-insensitive, not the name itself.
fn local_target(host_port: &str) -> Option<(&str, u16)> {
    let (host, port) = host_port.rsplit_once(':')?;
    let name = host.trim_end_matches('.');
    if !ends_with_local_suffix(name) {
        return None;
    }
    let port: u16 = port.parse().ok()?;
    Some((name, port))
}

/// Whether `name` ends with [`LOCAL_SUFFIX`], comparing ASCII letters
/// case-insensitively.
///
/// **Byte-sliced, not `str`-sliced.** [`LOCAL_SUFFIX`] is pure ASCII, but
/// `name` is caller-controlled and indexing a `str` at a byte offset that
/// falls inside a multi-byte character panics; slicing the `&[u8]` instead
/// never can, because a byte slice has no character-boundary constraint to
/// violate.
fn ends_with_local_suffix(name: &str) -> bool {
    let bytes = name.as_bytes();
    let suffix = LOCAL_SUFFIX.as_bytes();
    bytes.len() >= suffix.len() && bytes[bytes.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

/// One mDNS A query for `name`, its answer paired with `port`.
fn query_local(name: &str, port: u16) -> Result<Vec<SocketAddr>, String> {
    let mut resolver = OneShotMdnsResolver::new()
        .map_err(|e| format!("cannot open an mDNS resolver to look up {name}: {e}. Nothing has been sent."))?;
    resolver.set_query_timeout(QUERY_TIMEOUT);
    // **Ask for a MULTICAST answer, not a unicast one.** `simple-mdns` defaults
    // the query's QU (unicast-response) bit ON, and sends from one socket while
    // it reads the reply from a SEPARATE socket that is the one joined to the
    // multicast group. A unicast reply comes back to the sending socket's port,
    // which the resolver never reads — so the answer is silently lost and the
    // query times out. Proven live: fleetd on rivet-01 sent the query and
    // rivet-mdnsd on dev-01 was answering, yet this returned nothing until the
    // QU bit was cleared. `false` asks the responder to multicast its answer to
    // the group the reading socket is joined to, which is where it is heard.
    resolver.set_unicast_response(false);
    match resolver.query_service_address(name) {
        Ok(Some(ip)) => Ok(vec![SocketAddr::new(ip, port)]),
        Ok(None) => Err(format!(
            "{name} did not answer an mDNS query within {}s. A `.local` name is answered by a \
             `rivet-mdnsd` on the machine itself (PLAN.md 23), so either that machine is not \
             announcing - it is off, or has no DHCP lease yet - or this host is not on the same \
             link. Nothing has been sent.",
            QUERY_TIMEOUT.as_secs()
        )),
        Err(e) => Err(format!(
            "the mDNS query for {name} could not be made: {e}. Nothing has been sent."
        )),
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_local_name_with_a_port_is_an_mdns_target() {
        assert_eq!(local_target("dev-01.local:7777"), Some(("dev-01.local", 7777)));
    }

    /// An FQDN's trailing dot is stripped so the name matches what the
    /// responder published (`rivet_mdnsd::announce::fqdn` writes no trailing
    /// dot).
    #[test]
    fn a_trailing_dot_fqdn_is_still_a_local_target() {
        assert_eq!(local_target("dev-01.local.:7777"), Some(("dev-01.local", 7777)));
    }

    #[test]
    fn an_ip_literal_is_not_an_mdns_target() {
        assert_eq!(local_target("192.168.50.66:7777"), None);
    }

    #[test]
    fn a_real_dns_name_is_not_an_mdns_target() {
        assert_eq!(local_target("example.com:443"), None);
    }

    /// No port means this cannot build a `SocketAddr`, so it is the OS
    /// resolver's to refuse, not a target here.
    #[test]
    fn a_local_name_without_a_port_is_not_a_target() {
        assert_eq!(local_target("dev-01.local"), None);
    }

    #[test]
    fn an_unreadable_port_falls_through_rather_than_becoming_a_target() {
        assert_eq!(local_target("dev-01.local:notaport"), None);
    }

    /// A bracketed IPv6 literal is never mistaken for a `.local` name.
    #[test]
    fn a_bracketed_ipv6_literal_is_not_an_mdns_target() {
        assert_eq!(local_target("[::1]:7777"), None);
    }

    // -- the `.local` suffix is matched ASCII case-insensitively ------------

    /// **An uppercase suffix is still an mDNS target, and the name comes
    /// back exactly as written** — RFC 6762 §16 makes the comparison ASCII
    /// case-insensitive; it does not ask for the name to be folded.
    #[test]
    fn an_uppercase_local_suffix_is_still_an_mdns_target() {
        assert_eq!(local_target("dev-01.LOCAL:7777"), Some(("dev-01.LOCAL", 7777)));
    }

    #[test]
    fn a_mixed_case_local_suffix_is_still_an_mdns_target() {
        assert_eq!(local_target("dev-01.Local:7777"), Some(("dev-01.Local", 7777)));
    }

    /// The trailing-dot FQDN relaxation and the case relaxation both apply
    /// together, not only separately.
    #[test]
    fn an_uppercase_local_suffix_with_a_trailing_dot_is_still_a_target() {
        assert_eq!(local_target("dev-01.LOCAL.:7777"), Some(("dev-01.LOCAL", 7777)));
    }

    #[test]
    fn a_mixed_case_local_suffix_with_a_trailing_dot_is_still_a_target() {
        assert_eq!(local_target("dev-01.Local.:7777"), Some(("dev-01.Local", 7777)));
    }

    /// The whole point of the fall-through: a name that is not `.local`
    /// resolves exactly as it did before this crate existed. An IP literal is
    /// the case with no network dependency, so it is the one a hermetic test
    /// can prove.
    #[test]
    fn resolve_passes_an_ip_literal_straight_through() {
        let addresses = resolve("127.0.0.1:7777").expect("a loopback literal resolves");
        assert!(
            addresses
                .iter()
                .any(|a| a.port() == 7777 && a.ip().is_loopback()),
            "{addresses:?}"
        );
    }

    /// A non-`.local` host the OS refuses must be asked about once, and the
    /// error reported must be that first refusal — not a second, separately
    /// obtained one. Drives the seam directly so the count and the message are
    /// both visible without a real network lookup.
    #[test]
    fn the_os_lookup_runs_at_most_once_and_its_first_error_is_kept() {
        use std::cell::Cell;
        use std::io::Error;

        let calls = Cell::new(0u32);
        let result = resolve_with("example.com:443", |_| {
            let attempt = calls.get() + 1;
            calls.set(attempt);
            Err(Error::other(format!("refused on attempt {attempt}")))
        });

        assert_eq!(calls.get(), 1, "the OS lookup ran more than once");
        let message = result.expect_err("a refused OS lookup on a non-.local host is an error");
        assert!(message.contains("refused on attempt 1"), "{message}");
        assert!(!message.contains("attempt 2"), "{message}");
    }

    /// **A failing OS resolver still reaches the mDNS branch for every case
    /// variant of `.local`** — proven through the branch's OWN error, since
    /// no responder exists in this test's network. The point is which branch
    /// ran, not that a query was answered: if `local_target` had refused the
    /// name, `resolve_with` would report the OS resolver's own injected
    /// refusal below instead, exactly as
    /// [`the_os_lookup_runs_at_most_once_and_its_first_error_is_kept`] proves
    /// for a name that is genuinely not `.local`.
    #[test]
    fn resolve_with_takes_the_mdns_branch_for_every_case_variant_of_local() {
        let injected_refusal = "the OS resolver's own distinct refusal, never a real lookup";
        for host_port in
            ["dev-01.local:7777", "dev-01.LOCAL:7777", "dev-01.Local:7777", "dev-01.LOCAL.:7777"]
        {
            let result = resolve_with(host_port, |_| Err(std::io::Error::other(injected_refusal)));
            let message = result.expect_err("no responder exists, so this must still be an error");
            assert!(
                !message.contains(injected_refusal),
                "{host_port}: the mDNS branch must have run rather than reporting the OS \
                 resolver's own refusal: {message}"
            );
        }
    }
}
