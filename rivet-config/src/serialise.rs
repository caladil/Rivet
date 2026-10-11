//! [`Document`] out, TOML text in — the other half of `crate::parse`.
//!
//! `ADR-007` §3.1 splits the store from the representation and puts one
//! requirement on the representation:
//!
//! > The **representation** is generated text, in one format, canonically
//! > serialised so that two machines with the same state produce byte-identical
//! > documents.
//!
//! That is a *property*, and it is testable rather than aspirational. The
//! module's tests assert it three ways, because "byte-identical" fails for three
//! different reasons and each has its own guard:
//!
//! 1. **The same value always renders the same bytes**, however the document
//!    that produced it was written — different key order, comments, whitespace,
//!    `128M` against `134217728`, an empty `[services]` table against no table
//!    at all.
//! 2. **Rendering is a total function of the value and of nothing else.** No
//!    clock, no hostname, no build identifier reaches the output — a document is
//!    a statement of desired state and nothing observed may be written into it
//!    (§3.3).
//! 3. **Every rendered document parses back to the value it came from**, and
//!    renders identically a second time. A round trip that stopped at "parses"
//!    would miss a renderer that emits a *different* valid document each pass.
//!
//! # Why this lives here and not in `rivet-store`
//!
//! `rivet-store` is the crate that writes the bytes to `/state/config`, and it
//! is the obvious home until you ask who else needs them. `rivet export`
//! (`ADR-006` §6.2) renders a document to a stream with no filesystem in sight,
//! and a future API renders one to a socket. If the renderer lived in the
//! persistence adapter, either those callers would depend on a persistence
//! adapter to format a value, or there would be a second renderer — and two
//! renderers of one schema is `ADR-011` §3 exactly, with the additional property
//! that the disagreement would be silent and byte-level.
//!
//! So the schema crate owns both directions of its own text form, for the same
//! reason `crate::parse` is here: **turning bytes into types and types into
//! bytes is not I/O**, and `ADR-014` §3.1 keeps this crate free of it.
//!
//! # What is derived rather than quoted
//!
//! The record rules the *format* (TOML, `ADR-007` §6 item 2) and the *property*
//! (byte-identical, §3.1). It does not spell out the layout, and the three
//! choices below are therefore derived. They are marked as such so the
//! maintainer can overrule any of them cheaply — nothing outside this module
//! depends on which was taken.
//!
//! * **Key order follows the schema's own declared order**, not alphabetical
//!   order: `version`, `services`, `network` at the top level, and each entry's
//!   `DECLARED_KEYS` inside it. There is already exactly one list of each
//!   (`ADR-011`), the parser and the error messages already read it, and a
//!   second ordering rule here would be a second answer to "what order are the
//!   keys in".
//! * **Named tables are sorted by name**, which costs nothing because
//!   [`Document`] already holds them sorted for its own binary searches.
//! * **A header comment**, [`CANONICAL_HEADER`]. See below; this is the one that
//!   is a judgement rather than a derivation.
//!
//! ## The header, and its cost
//!
//! `ADR-007` §3.6.1 item 2 requires a provenance header on generated files and
//! says why it is the highest-value mechanism in its list: *"it is read by
//! exactly the person about to make the mistake: they have the file open."*
//! That section is written about `/etc`, and `/state/config` is not `/etc` — it
//! is the store itself, not a view of it.
//!
//! The mistake it prevents is nonetheless available, and only here.
//! `ADR-018` §4 makes `/state` the one writable filesystem on the machine, so
//! the stored document is the one Rivet file an operator can open and change
//! and have it take effect — which `ADR-007` §3.6.2 rules is not how anything is
//! changed. A file that can be edited with effect, with nothing in it saying so,
//! is the gap §3.6 exists to close.
//!
//! **The cost, stated rather than discovered:** the header is part of the
//! canonical bytes, so changing its text changes the bytes of every document on
//! every machine. It is a constant, it carries no per-machine and no per-build
//! content — a header with a timestamp or a version string in it would break
//! §3.1 outright — and `the_header_carries_nothing_that_varies` is the test
//! that keeps it that way.

use crate::document::Document;
use crate::network::Addressing;

/// Two lines at the top of every document Rivet writes.
///
/// See the module documentation for why it is here and what it costs. It must
/// stay a constant with no machine-varying content; the tests enforce both.
pub const CANONICAL_HEADER: &str = "\
# Rivet's configuration document. Written by Rivet and canonically
# serialised (ADR-007 §3.1); change it with the rivet tools, not an editor.
";

/// Render a document as the one text that represents it.
pub(crate) fn document(doc: &Document) -> String {
    let mut out = String::with_capacity(256);
    out.push_str(CANONICAL_HEADER);
    // `version` first, and required: `ADR-007` §7.1. Rendering it after a table
    // header would put it inside that table, which is a TOML fact rather than a
    // style choice.
    out.push_str(&format!("version = {}\n", doc.version()));

    // **Before any `[table]` header, and that is a fact about TOML rather than
    // a style.** A bare key written after a header belongs to that table, so a
    // `hostname` rendered at the end would come back as `volumes.hostname` and
    // the document would not mean what it said. The same reason `version` is
    // first.
    if let Some(hostname) = doc.hostname() {
        out.push_str(&format!("hostname = {}\n", basic_string(hostname.as_str())));
    }

    // **Beside `hostname`, and above every `[table]` header for its reason.**
    // A bare key written after a header belongs to that header's table, so a
    // `timezone` rendered at the end would come back as `kiosk.timezone` and
    // the document would refuse to read itself. The round-trip property test is
    // what would catch it; the placement is what stops it happening.
    if let Some(timezone) = doc.timezone() {
        out.push_str(&format!("timezone = {}\n", basic_string(timezone.as_str())));
    }

    // **`mode` and `fleet-role`, on `hostname`'s footing exactly** — added
    // 2026-09-13 (plan §5.2 item 2, `.rulings/` R-162). Both are bare
    // top-level scalars and must render before the first `[table]` header for
    // the identical TOML reason `hostname` and `timezone` already carry in
    // their own comments above.
    if let Some(mode) = doc.mode() {
        out.push_str(&format!("mode = {}\n", basic_string(mode.as_str())));
    }
    if let Some(fleet_role) = doc.fleet_role() {
        out.push_str(&format!("fleet-role = {}\n", basic_string(fleet_role.as_str())));
    }

    for service in doc.services() {
        out.push_str(&format!("\n[services.{}]\n", key(service.name().as_str())));
        out.push_str(&format!("enabled = {}\n", service.enabled()));
        // ~~`listen`~~ — **gone 2026-08-24**, with the key (the maintainer's 2026-08-24 ruling
        // item 2). A canonical document cannot render a key the schema no
        // longer has; a machine that emitted one would be writing a document
        // this build refuses to read back.
        if let Some(memory) = service.memory() {
            // A *string*, because that is what the schema declares and what
            // `crate::parse::read_memory` accepts. Rendering the byte count as
            // a TOML integer would produce a document this build refuses to
            // read, which the round-trip test is what catches.
            out.push_str(&format!("memory = {}\n", basic_string(&memory.to_string())));
        }
        // `memory`'s footing exactly, for `cpu` — `ADR-007` §7 and
        // `ADR-015` §3's 2026-09-09 amendment.
        if let Some(cpu) = service.cpu() {
            out.push_str(&format!("cpu = {}\n", basic_string(&cpu.to_string())));
        }
        // **Only when true** — `ADR-045` §4's default is false, and rendering
        // `critical = false` on every service would put a key an operator never
        // wrote into every document this machine emits.
        if service.critical() {
            out.push_str("critical = true\n");
        }
    }

    render_network(&mut out, doc);
    render_volumes(&mut out, doc);
    render_repos(&mut out, doc);
    render_control_plane(&mut out, doc);
    render_splash(&mut out, doc);
    render_kiosk(&mut out, doc);
    render_display(&mut out, doc);
    render_applications(&mut out, doc);
    render_carry(&mut out, doc);
    render_dns(&mut out, doc);

    out
}

/// The `[dns]` table and its records — `crate::dns`, `ADR-047` §8.
///
/// **The TTLs are written only when the document wrote them**, on
/// `[control-plane]`'s argument: an absent TTL and one naming the default are
/// two values, and writing the default in would turn *nobody said* into *the
/// operator chose 300*. Records follow in label order.
fn render_dns(out: &mut String, doc: &Document) {
    let Some(dns) = doc.dns() else {
        return;
    };
    out.push_str(&format!("\n[dns]\nzone = {}\n", basic_string(dns.zone().as_str())));
    if let Some(ttl) = dns.ttl() {
        out.push_str(&format!("ttl = {}\n", ttl.seconds()));
    }
    if let Some(ttl) = dns.negative_ttl() {
        out.push_str(&format!("negative-ttl = {}\n", ttl.seconds()));
    }
    for record in dns.records() {
        out.push_str(&format!("\n[dns.records.{}]\n", key(record.label().as_str())));
        out.push_str(&format!(
            "address = {}\n",
            basic_string(&record.address().to_string())
        ));
        if let Some(ttl) = record.ttl() {
            out.push_str(&format!("ttl = {}\n", ttl.seconds()));
        }
    }
}

/// The `[network]` table, one sub-table per interface, in the document's own
/// canonical (sorted) order.
fn render_network(out: &mut String, doc: &Document) {
    for interface in doc.network() {
        out.push_str(&format!("\n[network.{}]\n", key(interface.name().as_str())));
        out.push_str(&format!(
            "method = {}\n",
            basic_string(interface.addressing().method().as_str())
        ));
        // The `Addressing` enum has no variant holding both, so there is
        // nowhere for a contradictory document to be rendered *from* — the same
        // gate `crate::network` describes, read from the other direction.
        match interface.addressing() {
            Addressing::Dhcp => {}
            Addressing::Static { address, gateway } => {
                out.push_str(&format!("address = {}\n", basic_string(&address.to_string())));
                if let Some(gateway) = gateway {
                    out.push_str(&format!("gateway = {}\n", basic_string(&gateway.to_string())));
                }
            }
        }
        // **Last, and after `method`.** The order is the schema's, and
        // `ADR-007` §3.1's identical-state-identical-bytes rule makes it a
        // fact about the text rather than a preference.
        //
        // `basic_string` escapes it, which matters more here than anywhere
        // else in this file: an SSID is the one value in a Rivet document
        // chosen by somebody other than the operator, and a network named
        // `"` exists because somebody thought it was funny.
        if let Some(ssid) = interface.ssid() {
            out.push_str(&format!("ssid = {}\n", basic_string(ssid.as_str())));
        }
    }

    // **After every interface, because it is not one.** `[network.resolver]`
    // is the reserved sub-table (`crate::resolver`), and putting it last keeps
    // the interfaces contiguous and in their own sorted order — a resolver
    // sorted in among them by name would move when an interface was renamed.
    //
    // **A silent table renders nothing**, like `[services]` and `[network]`:
    // `ADR-007` §3.1 makes two documents meaning the same thing produce the
    // same bytes, and an empty table states nothing a missing one does not.
    if let Some(resolver) = doc.resolver().filter(|r| !r.is_silent()) {
        out.push_str("\n[network.resolver]\n");
        if !resolver.nameservers().is_empty() {
            let list: Vec<String> = resolver
                .nameservers()
                .iter()
                .map(|address| basic_string(&address.to_string()))
                .collect();
            out.push_str(&format!("nameservers = [{}]\n", list.join(", ")));
        }
        if !resolver.search().is_empty() {
            let list: Vec<String> = resolver
                .search()
                .iter()
                .map(|domain| basic_string(domain.as_str()))
                .collect();
            out.push_str(&format!("search = [{}]\n", list.join(", ")));
        }
    }
}


/// The `[volumes]` table, one sub-table per volume, in the document's own
/// canonical (sorted) order. `ADR-031` §7.1.
///
/// **An empty list renders nothing**, like `[services]` and `[network]`:
/// `ADR-031` §7.5 makes a declaration a statement about what should be attached,
/// so declaring none and saying nothing are one value and canonical form picks
/// silence.
fn render_volumes(out: &mut String, doc: &Document) {
    for volume in doc.volumes() {
        out.push_str(&format!("\n[volumes.{}]\n", key(volume.name().as_str())));
        out.push_str(&format!("guid = {}\n", basic_string(volume.guid().as_str())));
    }
}

/// The `[repos]` table, one sub-table per repository, in the document's own
/// canonical (sorted) order.
///
/// **Written in the same pass as the parser** — a table added without a
/// renderer can still pass every round-trip test, because no fixture carries
/// it (`PLAN.md` §39). The fixtures for this one arrive WITH it.
fn render_repos(out: &mut String, doc: &Document) {
    for repo in doc.repos() {
        out.push_str(&format!("\n[repos.{}]\n", key(repo.name().as_str())));
        out.push_str(&format!("id = {}\n", basic_string(repo.id().as_str())));
    }
}

/// The `[carry]` table, one sub-table per product, in the document's own
/// canonical (sorted) order — `crate::carry`, `.rulings/` R-159 R20.
///
/// **Written last**, matching `Document::DECLARED_KEYS`'s order, and on
/// `render_repos`'s footing: written in the same change as the parser, so no
/// fixture reaches this table without a renderer already in place for it.
fn render_carry(out: &mut String, doc: &Document) {
    for carry in doc.carry() {
        out.push_str(&format!("\n[carry.{}]\n", key(carry.product().as_str())));
        out.push_str(&format!("version = {}\n", basic_string(carry.version())));
    }
}

/// The `[applications.<name>]` tables — what each application is set to.
/// `ADR-038` §4, added 2026-09-02.
///
/// **Written only for what the document said**, like every other optional
/// table. Applications and their keys are both in canonical order, which
/// `crate::applications` holds them in for `ADR-007` §3.1's reason: two
/// machines in one state must produce byte-identical documents, and a TOML
/// table has no order a parser is obliged to preserve.
///
/// # What is NOT written back
///
/// Whatever `read_applications` could not carry — a value that is not a string,
/// a nested table. It is in `Applications::ignored` and it is reported; it is
/// not written, because writing it back would mean this crate had understood
/// it. `ADR-038` §4.4 makes that a report rather than a refusal, and a report
/// is not a promise to preserve.
///
/// **An application named with an empty table IS written**, because opening one
/// is a statement and a document that dropped it would not round-trip.
fn render_applications(out: &mut String, doc: &Document) {
    for settings in doc.applications().all() {
        out.push_str(&format!("\n[applications.{}]\n", key(settings.name())));
        // **`setting_key`, not `key`** — the loop variable would otherwise
        // shadow the `key()` helper for the rest of this block, and a call
        // written `key(key)` inside that shadow is not this function at all.
        // `crate::applications::parse_only`'s own fixtures are why this
        // needs escaping in the first place: a quoted key such as
        // `"unit.name"` is one setting, and written bare here it becomes a
        // dotted key — two nested tables, not the name that was parsed.
        for (setting_key, value) in settings.values() {
            out.push_str(&format!("{} = {}\n", key(setting_key), basic_string(value)));
        }
    }
}

/// The `[display]` table - the hours the screen is lit. Added 2026-09-02,
/// `ADR-039` §3.1.
///
/// **Written only when the document said something**, like every other optional
/// table here, and the pair is always written together because
/// `rivet_core::display::Hours` cannot hold half of one - which is why this
/// renderer has no branch and `ADR-007` §3.1's byte-identical requirement holds
/// for free.
///
/// The times render through `TimeOfDay`'s `Display`, which is canonical `HH:MM`
/// - so a document written `07:00` and a document that somehow arrived with a
/// different spelling of the same minute come back identical.
fn render_display(out: &mut String, doc: &Document) {
    let Some(hours) = doc.display() else {
        return;
    };
    out.push_str(&format!(
        "
[display]
off = {}
on = {}
",
        basic_string(&hours.off().to_string()),
        basic_string(&hours.on().to_string())
    ));
}

/// The `[splash]` table — what the boot screen may say, added 2026-08-30.
///
/// **Written only when the document SAID something**, like every other optional
/// table here. A machine that never mentioned the splash gets both lines, and
/// rendering `endpoint = true` for it would turn a silence into a statement —
/// after which two documents meaning the same thing would produce different
/// bytes, which is exactly what `ADR-007` §3.1 forbids.
fn render_splash(out: &mut String, doc: &Document) {
    let Some(splash) = doc.splash() else {
        return;
    };
    out.push_str(&format!(
        "\n[splash]\nendpoint = {}\nversion = {}\n",
        splash.endpoint(),
        splash.version()
    ));
}

/// The `[kiosk]` table — which application holds the display. Added 2026-08-31.
///
/// **Written only when it says something.** A `[kiosk]` that named nothing and
/// an absent `[kiosk]` mean the same thing to a supervisor, so rendering an
/// empty table would make two equal documents produce different bytes —
/// `ADR-007` §3.1.
fn render_kiosk(out: &mut String, doc: &Document) {
    let Some(kiosk) = doc.kiosk().filter(|k| !k.is_silent()) else {
        return;
    };
    out.push_str("\n[kiosk]\n");
    if let Some(name) = kiosk.auto_run() {
        // **`basic_string`, not a hand-wrapped `"{name}"`.** `read_kiosk`
        // accepts any string TOML can carry for `auto-run` — it is
        // `item.as_str()` with no character check — so a name containing a
        // `"` or a `\` is accepted input and this render must escape it or
        // produce TOML this same build refuses to parse back.
        out.push_str(&format!("auto-run = {}\n", basic_string(name)));
    }
    // **`locked` with no `auto-run` is written back too**, exactly as it was
    // authored: it is a contradiction an operator can state, and losing it
    // silently on the next edit would be worse than handing it back to them.
    // The machine resolves it visibly, on the screen.
    if kiosk.locked() {
        out.push_str("locked = true\n");
    }
    if let Some(name) = kiosk.launcher() {
        out.push_str(&format!("launcher = {}\n", basic_string(name)));
    }
    if let Some(name) = kiosk.compositor() {
        out.push_str(&format!("compositor = {}\n", basic_string(name)));
    }
}

/// The `[control-plane]` table — `ADR-035` §7.2, added 2026-08-28.
///
/// **Absent when the operator said nothing**: an absent table and one naming
/// the default address are two different values, and `ADR-007` §3.1 requires
/// a document read and written back to be byte-identical. Writing the default
/// in would turn *nobody said* into *the operator chose loopback*, which is
/// the difference a later change to that default would turn into a surprise.
fn render_control_plane(out: &mut String, doc: &Document) {
    let Some(control_plane) = doc.control_plane() else {
        return;
    };
    out.push_str("
[control-plane]
");
    out.push_str(&format!("listen = {}
", basic_string(control_plane.listen())));
}

/// A TOML key: bare where the grammar allows it, quoted where it does not.
///
/// **This is not decoration and the quoting is not optional.** A service name
/// may contain `.` (`crate::services::ServiceName`) and an interface name may
/// contain `.`, `"` or `\` — the kernel's `dev_valid_name()` forbids only `/`,
/// `:`, whitespace and NUL, so `br0.100` is an ordinary VLAN interface and a
/// perfectly legal key. Written bare, `[network.br0.100]` is three nested
/// tables and the document does not mean what it said.
fn key(name: &str) -> String {
    let bare = !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if bare {
        name.to_owned()
    } else {
        basic_string(name)
    }
}

/// A TOML basic string, escaped per the 1.0 grammar.
///
/// Every value this module renders comes from a validated type, so most of this
/// is unreachable from a document that parsed. It is written in full anyway:
/// the alternative is a renderer that is correct only for as long as nobody
/// widens a type's admissible characters, and the failure would be a document
/// that no longer parses on the machine that just wrote it.
fn basic_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\u{:04X}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::document::SCHEMA_VERSION;

    fn parse(src: &str) -> Document {
        Document::parse(src).expect("the fixture is a valid document")
    }

    /// A passphrase-shaped value, so the output can be checked for leakage.
    /// `AGENTS.md` §8.3 — a test fixture, not a real secret.
    const NOT_A_REAL_SECRET: &str = "hunter2-correct-horse-battery";

    /// `ADR-007` §7.3's own example, written out here as bytes rather than
    /// produced by the code under test.
    ///
    /// **Provenance matters and this is the weaker kind.** `AGENTS.md` §4:
    /// a round trip of a reader against its own writer proves the two agree and
    /// nothing about the format. This literal is hand-composed from the ADR, so
    /// it proves the renderer agrees with the *record*; what it cannot prove is
    /// that a third-party TOML parser reads it the same way, and no such parser
    /// is in this workspace to ask.
    const ADR_007_7_3_EXAMPLE: &str = "\
# Rivet's configuration document. Written by Rivet and canonically
# serialised (ADR-007 §3.1); change it with the rivet tools, not an editor.
version = 1

[network.eth0]
method = \"static\"
address = \"192.168.1.50/24\"
gateway = \"192.168.1.1\"

[network.wlan0]
method = \"dhcp\"
";

    // ---- the bytes ---------------------------------------------------------

    #[test]
    fn the_adr_007_example_renders_to_the_bytes_written_down_here() {
        let doc = parse(
            "version = 1\n\
             [network.eth0]\nmethod = \"static\"\naddress = \"192.168.1.50/24\"\n\
             gateway = \"192.168.1.1\"\n\
             [network.wlan0]\nmethod = \"dhcp\"\n",
        );
        assert_eq!(doc.to_toml(), ADR_007_7_3_EXAMPLE);
    }

    #[test]
    fn an_empty_document_is_the_header_and_a_version() {
        let expected = format!("{CANONICAL_HEADER}version = {SCHEMA_VERSION}\n");
        assert_eq!(Document::default().to_toml(), expected);
    }

    #[test]
    fn every_document_ends_in_exactly_one_newline() {
        for src in [
            "version = 1\n",
            "version = 1\n[services.a]\nenabled = false\n",
            "version = 1\n[network.eth0]\nmethod = \"dhcp\"\n",
        ] {
            let text = parse(src).to_toml();
            assert!(text.ends_with('\n'), "{text:?}");
            assert!(!text.ends_with("\n\n"), "{text:?}");
        }
    }

    // ---- `ADR-007` §3.1: two machines in the same state, identical bytes ----

    /// The property, stated as the ADR states it. Five spellings of one state —
    /// key order, comments, whitespace, an empty table against no table, and
    /// two spellings of one memory limit — must all produce the same bytes.
    #[test]
    fn documents_written_five_different_ways_render_to_one_byte_sequence() {
        let spellings = [
            "version = 1\n\
             [services.rivet-apid]\nenabled = true\nmemory = \"128M\"\n\
             [network.eth0]\nmethod = \"dhcp\"\n",
            // The tables in the other order, and the keys inside them too.
            // `version` stays at the top because a bare key after a `[table]`
            // header belongs to that table — a fact about TOML, not a style.
            "version = 1\n\
             [network.eth0]\nmethod = \"dhcp\"\n\
             [services.rivet-apid]\nmemory = \"128M\"\nenabled = true\n",
            // Comments and blank lines everywhere.
            "# a comment\nversion = 1   # another\n\n\n\
             [services.rivet-apid]\n# and here\nenabled = true\nmemory = \"128M\"\n\
             [network.eth0]\nmethod = \"dhcp\"\n",
            // The same memory limit, spelled in bytes.
            "version = 1\n\
             [services.rivet-apid]\nenabled = true\nmemory = \"134217728\"\n\
             [network.eth0]\nmethod = \"dhcp\"\n",
            // Dotted keys and inline tables — TOML's other spellings.
            "version = 1\n\
             services.rivet-apid = { enabled = true, memory = \"128M\" }\n\
             network.eth0 = { method = \"dhcp\" }\n",
        ];
        let rendered: Vec<String> = spellings.iter().map(|s| parse(s).to_toml()).collect();
        for (i, text) in rendered.iter().enumerate() {
            assert_eq!(
                text, &rendered[0],
                "spelling {i} renders differently:\n{text}\n--- against ---\n{}",
                rendered[0]
            );
        }
    }

    /// An empty `[services]` table and no table at all are the same *value* —
    /// `crate::parse::read_services` returns an empty vector for both — so
    /// canonical form has to choose one spelling. It chooses silence.
    #[test]
    fn an_empty_table_and_an_absent_one_render_the_same_and_render_nothing() {
        let a = parse("version = 1\n[services]\n[network]\n").to_toml();
        let b = parse("version = 1\n").to_toml();
        assert_eq!(a, b);
        assert!(!a.contains("services"), "{a}");
        assert!(!a.contains("network"), "{a}");
    }

    /// Rendering must be a function of the value alone. Called twice, a
    /// hundred milliseconds apart in wall-clock terms, it must not differ —
    /// which is the cheap form of "no clock, no hostname, no build id reached
    /// the output" (`ADR-007` §3.3).
    #[test]
    fn the_header_carries_nothing_that_varies() {
        let doc = parse("version = 1\n[network.eth0]\nmethod = \"dhcp\"\n");
        assert_eq!(doc.to_toml(), doc.to_toml());
        // And it names nothing about this machine or this build.
        for forbidden in [env!("CARGO_PKG_VERSION"), "20", "T0"] {
            assert!(
                !CANONICAL_HEADER.contains(forbidden),
                "the header carries {forbidden:?}, which varies: {CANONICAL_HEADER:?}"
            );
        }
    }

    // ---- round trips -------------------------------------------------------

    /// Parse, render, parse: the value must survive, **and so must the bytes.**
    /// The second render is what catches a renderer that emits a different
    /// valid document each pass.
    #[test]
    fn every_document_round_trips_by_value_and_by_bytes() {
        let cases = [
            "version = 1\n",
            "version = 1\n[services.rivet-apid]\nenabled = true\n",
            "version = 1\n[services.rivet-apid]\nenabled = false\n",
            "version = 1\n[services.a]\nenabled = true\nmemory = \"1G\"\n\
             [services.b]\nenabled = false\nmemory = \"1023\"\n",
            "version = 1\n[network.eth0]\nmethod = \"static\"\naddress = \"10.0.0.2/24\"\n",
            "version = 1\n[network.eth0]\nmethod = \"static\"\naddress = \"10.0.0.2/24\"\n\
             gateway = \"10.0.0.1\"\n[network.wlan0]\nmethod = \"dhcp\"\n",
            // Keys the TOML grammar cannot carry bare.
            "version = 1\n[network.\"br0.100\"]\nmethod = \"dhcp\"\n",
            "version = 1\n[services.\"a.b.c\"]\nenabled = true\n",
            // `[volumes]`, including an uppercase GUID that must come back
            // lowercased and two entries that must come back name-sorted.
            "version = 1\n[volumes.bulk]\nguid = \"1a2b3c4d-5e6f-4071-8293-a4b5c6d7e8f9\"\n",
            "version = 1\n[volumes.media]\nguid = \"1A2B3C4D-5E6F-4071-8293-A4B5C6D7E8F9\"\n\
             [volumes.archive]\nguid = \"9f8e7d6c-5b4a-4392-8170-6f5e4d3c2b1a\"\n",
            // **`hostname`, and the reason these are here is a lesson a table
            // taught this file once: a key that ships in the parser, the
            // schema and the /etc projection with NO renderer passes every
            // test here as long as no fixture carries it (PLAN.md 37.5). A
            // scalar key is even easier to drop silently, so it arrives with
            // its own.
            "version = 1\nhostname = \"kitchen-pi\"\n",
            // With tables after it, which is what catches a renderer that puts
            // a bare key below a [header] and changes what it belongs to.
            "version = 1\nhostname = \"kitchen-pi\"\n[network.eth0]\nmethod = \"dhcp\"\n",
            // **`timezone`, and it arrives with its renderer for `hostname`'s
            // reason** — a scalar key is the easiest thing in this crate to
            // parse, validate, schema-publish and then never write back out.
            "version = 1\ntimezone = \"America/Chicago\"\n",
            // **`[applications.<name>]` — `ADR-038` §4**, the region whose schema
            // this crate does not own. A table with no renderer is dropped in
            // silence, and this one would be dropped from exactly the
            // documents that configure a fleet.
            "version = 1\n[applications.aura-renderer]\nbrightness = \"60\"\n",
            // Two applications and two keys each, which pins the CANONICAL
            // ORDER: both levels are written sorted, and a document read and
            // written back must be byte-identical (`ADR-007` §3.1).
            "version = 1\n[applications.aura-renderer]\nbackground = \"digital-rain\"\n\
             brightness = \"60\"\n[applications.aura-vitals]\ntick = \"15\"\n",
            // An application named with an EMPTY table: opening one is a
            // statement, and a document that dropped it would not round-trip.
            "version = 1\n[applications.aura-renderer]\n",
            // **`[display]` - the hours the screen is lit, `ADR-039` §3.1.**
            // A table with no renderer is dropped in silence, and this one
            // would be dropped from the documents that switch a building's
            // screens off.
            "version = 1\n[display]\noff = \"19:00\"\non = \"07:00\"\n",
            // A window that does NOT cross midnight, so the round trip does not
            // depend on the ordering the common case happens to have.
            "version = 1\n[display]\noff = \"02:00\"\non = \"07:00\"\n",
            // Midnight at each end, where an off-by-one in the canonical
            // `HH:MM` spelling would show up.
            "version = 1\n[display]\noff = \"00:00\"\non = \"12:00\"\n",
            "version = 1\n[display]\noff = \"12:00\"\non = \"00:00\"\n",
            // With the timezone it is read against, and with `[kiosk]`, so the
            // ORDER of the tables is pinned the way the two scalars' is.
            "version = 1\ntimezone = \"America/Chicago\"\n\
             [display]\noff = \"19:00\"\non = \"07:00\"\n",
            "version = 1\n[kiosk]\nauto-run = \"aura-renderer\"\n\
             [display]\noff = \"19:00\"\non = \"07:00\"\n",
            // Both scalars at once, which is what fixes their ORDER: a
            // canonical document has one representation, so `hostname` before
            // `timezone` is part of the format rather than an accident.
            "version = 1\nhostname = \"kitchen-pi\"\ntimezone = \"Europe/Berlin\"\n",
            // With a table after it — the bare-key-under-a-header trap again,
            // which for this key would come back as `network.timezone`.
            "version = 1\ntimezone = \"UTC\"\n[network.eth0]\nmethod = \"dhcp\"\n",
            // **`mode` and `fleet-role` — plan §5.2 item 2, `.rulings/` R-162.**
            // Here for `hostname`'s reason: two more scalars a renderer could
            // silently drop.
            "version = 1\nmode = \"kiosk\"\n",
            "version = 1\nfleet-role = \"server\"\n",
            // Both at once, which fixes their ORDER against each other and
            // against `hostname`/`timezone` — a canonical document has one
            // representation.
            "version = 1\nhostname = \"kitchen-pi\"\nmode = \"kiosk\"\nfleet-role = \"server\"\n\
             timezone = \"America/Chicago\"\n",
            // With a table after it — the bare-key-under-a-header trap once
            // more, which for these keys would come back as
            // `control-plane.mode`/`control-plane.fleet-role`.
            "version = 1\nfleet-role = \"member\"\n[control-plane]\nlisten = \"0.0.0.0:7777\"\n",
            // **`[repos]`, and these arrive WITH the renderer** — a table
            // added without one does not get caught by anything else, so a
            // fixture that exercises it exists from the start (PLAN.md 37.5,
            // 39).
            "version = 1\n[repos.example-repo]\nid = \"4f2c9a01bd7e3865\"\n",
            // Two repositories, which must come back name-sorted rather than
            // in the order they were written.
            "version = 1\n[repos.zebra]\nid = \"4f2c9a01bd7e3865\"\n[repos.alpha]\nid = \"a1b2c3d4e5f60718\"\n",
            // Beside every other table, which is what catches an ordering slip -
            // `[repos]` renders after `[volumes]` and before `[control-plane]`.
            "version = 1\n[volumes.media]\nguid = \"6f2c9a01-bd7e-4865-9a1b-2c3d4e5f6071\"\n[repos.example-repo]\nid = \"4f2c9a01bd7e3865\"\n[control-plane]\nlisten = \"0.0.0.0:7777\"\n",
            // Every character the grammar allows. **This fixture is why `.` is
            // not one of them**: its first spelling was `a-b_c.d1`, and TOML
            // read that as a table inside a table rather than as a name.
            "version = 1\n[repos.a-b_c-d1]\nid = \"4f2c9a01bd7e3865\"\n",
            // **`[carry]`, `.rulings/` R-159 R20, arriving WITH its renderer**
            // on `[repos]`'s footing exactly, for the identical reason.
            "version = 1\n[carry.rivet-fleetd]\nversion = \"0.1.0\"\n",
            // Two pins, which must come back product-sorted rather than in the
            // order they were written.
            "version = 1\n[carry.rivet-proxyd]\nversion = \"0.2.0\"\n[carry.rivet-fleetd]\nversion = \"0.1.0\"\n",
            // Beside every other table — `[carry]` renders last, after
            // `[applications]`.
            "version = 1\n[applications.aura-renderer]\nbrightness = \"60\"\n[carry.rivet-fleetd]\nversion = \"0.1.0\"\n",
            // **`[dns]`, `ADR-047` §8, arriving WITH its renderer.** A zone
            // alone, with the TTLs omitted so the omission must survive.
            "version = 1\n[dns]\nzone = \"my-fleet.home.arpa\"\n",
            // Both TTLs, two records written out of order that must come back
            // label-sorted, and one record's own TTL.
            "version = 1\n[dns]\nzone = \"my-fleet.home.arpa\"\nttl = 300\nnegative-ttl = 60\n\
             [dns.records.arbor]\naddress = \"192.168.50.10\"\n\
             [dns.records.printer]\naddress = \"192.168.50.40\"\nttl = 60\n",
            // Beside `[carry]`, which renders before it.
            "version = 1\n[carry.rivet-fleetd]\nversion = \"0.1.0\"\n\
             [dns]\nzone = \"home.arpa\"\n[dns.records.a]\naddress = \"10.0.0.1\"\n",
        ];
        for src in cases {
            let first = parse(src);
            let text = first.to_toml();
            let second = Document::parse(&text)
                .unwrap_or_else(|e| panic!("{src:?} rendered to {text:?}, which will not parse: {e}"));
            assert_eq!(first, second, "value changed for {src:?}\n{text}");
            assert_eq!(text, second.to_toml(), "bytes changed on the second pass");
        }
    }

    // A test stood here until 2026-08-24: `listen = []` and no `listen` at all
    // were different states, so they had to render differently, and it was the
    // one place where rendering *less* would have been a silent change of
    // meaning. It is deleted with the key (the maintainer's 2026-08-24 ruling) rather than
    // adapted, because the three-way absence it protected existed only while
    // two authorities could each state a value.
    //
    /// **What replaces it**: a document can no longer render the key at all, and
    /// a renderer that started to would produce a document this build refuses to
    /// read back. That is what the round-trip test above already enforces for
    /// every key; this states it for the one that was removed, so a future
    /// re-addition is a deliberate act rather than an accident.
    #[test]
    fn no_rendered_document_carries_a_key_the_parser_would_refuse() {
        let rendered = parse(
            "version = 1\n[services.a]\nenabled = true\nmemory = \"64M\"\n\
             [services.b]\nenabled = false\n",
        )
        .to_toml();
        assert!(!rendered.contains("listen"), "{rendered}");
        Document::parse(&rendered).expect("a rendered document must parse back");
    }

    /// The interesting keys, driven through the parser rather than asserted on
    /// as strings: a name with a `.` in it is a real VLAN interface, and written
    /// bare it would become nested tables and change what the document says.
    #[test]
    fn an_awkward_key_is_quoted_and_reads_back_as_the_same_one_name() {
        for name in ["br0.100", "eth-0", "eth_0", "e\"0", "e\\0", "vlan#1"] {
            let src = format!("version = 1\n[network.\"{}\"]\nmethod = \"dhcp\"\n", escaped(name));
            let doc = Document::parse(&src)
                .unwrap_or_else(|e| panic!("{name:?} is a valid interface name: {e}"));
            let text = doc.to_toml();
            let back = Document::parse(&text)
                .unwrap_or_else(|e| panic!("{name:?} rendered to {text:?}, unparseable: {e}"));
            assert_eq!(back.network().len(), 1, "{text}");
            assert!(back.interface(name).is_some(), "{name:?} was lost in {text}");
        }
    }

    /// Helper for the fixture above: writing an awkward name into a *fixture*
    /// needs the same escaping the renderer does, and using the renderer's own
    /// function for it would make the test agree with itself by construction.
    fn escaped(name: &str) -> String {
        name.replace('\\', "\\\\").replace('"', "\\\"")
    }

    #[test]
    fn a_bare_key_is_left_bare_so_the_common_document_reads_normally() {
        assert_eq!(key("eth0"), "eth0");
        assert_eq!(key("rivet-netd"), "rivet-netd");
        assert_eq!(key("a_b"), "a_b");
        assert_eq!(key("br0.100"), "\"br0.100\"");
        assert_eq!(key(""), "\"\"");
    }

    #[test]
    fn a_basic_string_escapes_what_the_toml_grammar_requires() {
        assert_eq!(basic_string("a"), "\"a\"");
        assert_eq!(basic_string("a\"b"), "\"a\\\"b\"");
        assert_eq!(basic_string("a\\b"), "\"a\\\\b\"");
        assert_eq!(basic_string("a\nb"), "\"a\\nb\"");
        assert_eq!(basic_string("a\tb"), "\"a\\tb\"");
        assert_eq!(basic_string("a\rb"), "\"a\\rb\"");
        assert_eq!(basic_string("a\u{1}b"), "\"a\\u0001b\"");
        assert_eq!(basic_string("a\u{7f}b"), "\"a\\u007Fb\"");
    }

    // ---- B02: application names, setting keys and auto-run are escaped ------

    /// Helper for the fixtures below: independent of `basic_string`, on
    /// `escaped`'s own reasoning above extended to the two control characters
    /// this section's `auto-run` fixture needs and `escaped` does not.
    fn escaped_control(name: &str) -> String {
        name.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\t', "\\t")
            .replace('\n', "\\n")
    }

    /// **A quoted application name is quoted on the way out.** `["applications".
    /// "my.app"]` and `[applications.my.app]` name different things — the
    /// second is two nested tables — so a renderer that wrote the first name
    /// bare would produce a document that reads back as something else.
    #[test]
    fn a_quoted_application_name_is_quoted_on_the_way_out() {
        let doc = parse("version = 1\n[applications.\"my.app\"]\nbrightness = \"60\"\n");
        let text = doc.to_toml();
        assert!(
            text.contains("[applications.\"my.app\"]"),
            "the dotted name was written bare: {text}"
        );
        let again = Document::parse(&text)
            .unwrap_or_else(|e| panic!("rendered to {text:?}, which will not parse: {e}"));
        assert_eq!(doc, again, "value changed\n{text}");
        assert_eq!(text, again.to_toml(), "bytes changed on the second pass");
        assert_eq!(
            again.applications().get("my.app").and_then(|a| a.get("brightness")),
            Some("60"),
            "the application split into nested tables instead of round-tripping: {text}"
        );
    }

    /// **A flat setting key containing `.` or a space is quoted on the way
    /// out.** `crate::applications::parse_only` reads `"unit.name" = "x"` as
    /// one setting named `unit.name`; written bare, TOML reads
    /// `unit.name = "x"` as a DOTTED key — table `unit`, key `name` — and
    /// `ApplicationSettings::get("unit.name")` never sees it again.
    #[test]
    fn a_dotted_or_spaced_setting_key_is_quoted_on_the_way_out() {
        let doc = parse(
            "version = 1\n[applications.aura-renderer]\n\"unit.name\" = \"a\"\n\"a b\" = \"c\"\n",
        );
        let text = doc.to_toml();
        assert!(text.contains("\"unit.name\" = \"a\""), "{text}");
        assert!(text.contains("\"a b\" = \"c\""), "{text}");
        let again = Document::parse(&text)
            .unwrap_or_else(|e| panic!("rendered to {text:?}, which will not parse: {e}"));
        assert_eq!(doc, again, "value changed\n{text}");
        assert_eq!(text, again.to_toml(), "bytes changed on the second pass");
        let settings = again
            .applications()
            .get("aura-renderer")
            .expect("the application table survived");
        assert_eq!(settings.get("unit.name"), Some("a"), "{text}");
        assert_eq!(settings.get("a b"), Some("c"), "{text}");
    }

    /// **`auto-run` survives a quote, a backslash, a tab and a newline.**
    /// `read_kiosk` puts no character restriction on the value it accepts —
    /// `item.as_str()` alone — so all four are accepted input, and the old
    /// `auto-run = "{name}"` interpolation either changed what the name said
    /// or produced TOML this same build refuses to read back.
    #[test]
    fn an_auto_run_value_with_a_quote_backslash_tab_and_newline_round_trips() {
        let name = "weird\"app\\name\twith\nlines";
        let src = format!(
            "version = 1\n[kiosk]\nauto-run = \"{}\"\n",
            escaped_control(name)
        );
        let doc = Document::parse(&src).expect("a valid fixture");
        let text = doc.to_toml();
        let again = Document::parse(&text)
            .unwrap_or_else(|e| panic!("{src:?} rendered to {text:?}, which will not parse: {e}"));
        assert_eq!(doc, again, "value changed for {src:?}\n{text}");
        assert_eq!(text, again.to_toml(), "bytes changed on the second pass");
        assert_eq!(
            again.kiosk().and_then(|k| k.auto_run()),
            Some(name),
            "{text}"
        );
    }

    /// **The launcher and the compositor survive a canonical rewrite**, alone
    /// and beside the other two keys. A table naming only a launcher is not
    /// silent, and dropping it on the next `apply edit` would hand the role
    /// back to the image's own launcher without anybody asking.
    #[test]
    fn a_named_launcher_and_compositor_survive_a_canonical_rewrite() {
        for src in [
            "version = 1\n[kiosk]\nlauncher = \"my-launcher\"\n",
            "version = 1\n[kiosk]\ncompositor = \"cosmic-comp\"\n",
            "version = 1\n[kiosk]\nauto-run = \"aura-renderer\"\nlocked = true\n\
             launcher = \"my-launcher\"\ncompositor = \"cosmic-comp\"\n",
        ] {
            let doc = Document::parse(src).expect("a valid fixture");
            let text = doc.to_toml();
            let again = Document::parse(&text)
                .unwrap_or_else(|e| panic!("{src:?} rendered to {text:?}, which will not parse: {e}"));
            assert_eq!(doc, again, "value changed for {src:?}\n{text}");
            assert_eq!(text, again.to_toml(), "bytes changed on the second pass");
        }
        let doc = Document::parse("version = 1\n[kiosk]\nlauncher = \"my-launcher\"\n")
            .expect("a valid fixture");
        assert_eq!(doc.kiosk().and_then(|k| k.launcher()), Some("my-launcher"));
        assert_eq!(doc.kiosk().and_then(|k| k.compositor()), None);
    }

    // ---- the splash table ---------------------------------------------------

    /// **The failure this guards against is silent.** `apply edit` rewrites the
    /// document canonically, so a table the serialiser does not know about is
    /// not rejected — it is DROPPED, and a machine told to keep quiet about
    /// itself would start announcing again the next time anything else in the
    /// document changed.
    #[test]
    fn a_suppressed_splash_survives_a_canonical_rewrite() {
        let text = "version = 1\n[splash]\nendpoint = false\nversion = false\n";
        let doc = crate::parse::document(text).expect("a valid document");
        let written = document(&doc);
        let again = crate::parse::document(&written).expect("what we wrote must parse");
        assert_eq!(
            again.effective_splash(),
            crate::splash::Splash::new(false, false),
            "the splash settings did not survive a rewrite: {written}"
        );
    }

    /// One line off and one on, because a serialiser that wrote a single
    /// boolean for both would pass the test above.
    #[test]
    fn the_two_lines_round_trip_independently() {
        for (endpoint, version) in [(true, false), (false, true)] {
            let text = format!(
                "version = 1\n[splash]\nendpoint = {endpoint}\nversion = {version}\n"
            );
            let doc = crate::parse::document(&text).expect("valid");
            let again = crate::parse::document(&document(&doc)).expect("re-parses");
            assert_eq!(
                again.effective_splash(),
                crate::splash::Splash::new(endpoint, version),
                "endpoint={endpoint} version={version} did not survive"
            );
        }
    }

    /// **A document that said nothing must keep saying nothing.** Rendering the
    /// defaults would turn a silence into a statement, and then two documents
    /// meaning the same thing would produce different bytes — which is what
    /// `ADR-007` §3.1 forbids.
    #[test]
    fn a_document_with_no_splash_table_does_not_grow_one() {
        let doc = crate::parse::document("version = 1\n").expect("valid");
        let written = document(&doc);
        assert!(
            !written.contains("[splash]"),
            "a silence became a statement: {written}"
        );
        // And the effective answer is still both lines.
        assert_eq!(doc.effective_splash(), crate::splash::Splash::default());
    }

    /// The zero-scrub consequence (`AGENTS.md` §8.3, `ADR-020` §4) at the
    /// writing end rather than the diagnostic end: this is the function that
    /// puts bytes on the one writable filesystem, and the schema has no field
    /// that holds a credential. A document that could not carry one cannot
    /// write one.
    #[test]
    fn a_document_has_nowhere_to_put_a_secret_so_none_is_rendered() {
        // A passphrase offered under any key at all is refused at parse time,
        // so it never reaches a value the renderer could emit.
        let err = Document::parse(&format!("version = 1\npsk = \"{NOT_A_REAL_SECRET}\"\n"))
            .expect_err("the schema declares no such key");
        assert!(!err.to_string().contains(NOT_A_REAL_SECRET));
    }


    // ---- what must never be rendered ---------------------------------------

    /// `ADR-007` Â§3.3: the document says what the machine should be and never
    /// what it is. There is no field for observed state, so this is a guard on
    /// the *renderer* rather than on the schema â€” the value it is handed cannot
    /// carry a carrier, a MAC or a lease expiry, and the output must not
    /// acquire one on the way out.
    #[test]
    fn nothing_observed_reaches_the_output() {
        let doc = parse("version = 1\n[network.eth0]\nmethod = \"static\"\naddress = \"10.0.0.2/24\"\n");
        let text = doc.to_toml();
        for observed in ["carrier", "up", "down", "lease", "mac", "state"] {
            assert!(!text.contains(observed), "{observed:?} appears in\n{text}");
        }
    }
}
