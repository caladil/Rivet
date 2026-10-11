//! **The routing table `rivet-proxyd` serves from** — `R-133`, 2026-09-03.
//!
//! The maintainer's ruling (`.rulings/` R-133): the proxy routes by path and
//! only by path, for now at least, which spares the machine the name-resolution
//! trouble that per-application names would bring.
//!
//! So the machine has **one name and one certificate**, and every application
//! that asked for one is reached at `https://<host>/<path>/`.
//!
//! # Why this lives in `rivet-core` and not in the daemon
//!
//! `ADR-011`: one fact, one home. **Three** programs need the table's shape —
//! the supervisor that writes it, the proxy that reads it, and any tool that
//! wants to report what is routed — and a format defined in the daemon would
//! make the supervisor depend on a binary.
//!
//! It is the same arrangement as [`crate::settings`], deliberately: a view of
//! the desired state, rewritten every convergence, that an application reads
//! from its own data directory and never has to ask about.
//!
//! # The table is DERIVED, and that is the whole design
//!
//! Nothing here is configured. Every row comes from an installed application's
//! signed manifest — its `proxy-path`, its first TCP `listen` port, and what it
//! asks of a caller — so a route exists because an author asked for it in
//! something the machine verified, not because an operator typed it twice.
//!
//! # A route is not one answer about authentication — `ADR-048`
//!
//! What a caller must present is decided **per prefix**, so one application on
//! one listening socket can hold a gated page and an open door at once. The
//! whole-application form is the same declaration with no prefixes named, which
//! is why every manifest written before that ADR still means what it meant.
//!
//! [`Route::requires_operator_for`] is the answer a gate asks for, and it
//! compares **whole segments** and never decodes: an open `pair` does not open
//! `pairings`, `pair-admin`, `Pair` or `%70air`. That is not a detail of the
//! implementation — it is the property the mechanism is worth having for.
//!
//! **A path is a request and the document grants it** (`ADR-015` §3.3). Two
//! applications may ask for `/aura`; the manifest is signed and an operator
//! cannot edit it, so the collision is resolved here — by [`build`], which
//! **refuses the pair by name** rather than picking one. Picking would mean the
//! reachable application depended on install order.

/// **The setting naming the certificate the applications behind the proxy
/// present** — `ADR-046` §4, `ADR-060` §3. Without it, the first DNS name on
/// the proxy's own certificate.
///
/// Spelled here because two programs read it: `rivet-proxyd`, which verifies
/// its backends against it, and `rivet-mcpd`, which reaches the same backends
/// for a backup over the control plane (`ADR-051` §8) and must verify them
/// against the same name.
pub const UPSTREAM_NAME_SETTING: &str = "upstream-name";

/// **The setting that decides whether the proxy accepts the operator's client
/// certificate** — `ADR-046` §4, `ADR-063` §4, `.rulings/` R-234.
///
/// [`CERTIFICATE_ACCEPTED`] or [`CERTIFICATE_REFUSED`]. **Absent means
/// accepted**, so every machine whose document says nothing behaves exactly as
/// it did before the setting existed; a machine turns the certificate off by
/// its own document, once a passkey is enrolled there and the tools that used
/// the certificate have moved to the control plane.
pub const OPERATOR_CERTIFICATE_SETTING: &str = "operator-certificate";

/// [`OPERATOR_CERTIFICATE_SETTING`]'s value for a machine that accepts it.
pub const CERTIFICATE_ACCEPTED: &str = "accept";

/// [`OPERATOR_CERTIFICATE_SETTING`]'s value for a machine that no longer does.
pub const CERTIFICATE_REFUSED: &str = "refuse";

/// Whether the operator's client certificate admits a caller at the gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatorCertificate {
    /// It does, beside a passkey session — the default.
    Accepted,
    /// It does not: only a passkey session admits the operator.
    Refused,
}

/// **Read [`OPERATOR_CERTIFICATE_SETTING`]**, and what to journal about it.
///
/// Absent is [`OperatorCertificate::Accepted`]. **A value that is neither word
/// is [`OperatorCertificate::Refused`]**, with a sentence saying why: the
/// setting exists to stop accepting a credential, and a misspelling must not
/// leave it accepted while the operator believes it retired. Refusing it costs
/// a sign-in with a passkey; the front door stays up either way.
#[must_use]
pub fn operator_certificate(setting: Option<&str>) -> (OperatorCertificate, Option<String>) {
    match setting.map(str::trim) {
        None | Some(CERTIFICATE_ACCEPTED) => (OperatorCertificate::Accepted, None),
        Some(CERTIFICATE_REFUSED) => (OperatorCertificate::Refused, None),
        Some(other) => (
            OperatorCertificate::Refused,
            Some(format!(
                "the `{OPERATOR_CERTIFICATE_SETTING}` setting is {other:?}, which is neither \
                 {CERTIFICATE_ACCEPTED:?} nor {CERTIFICATE_REFUSED:?}. The operator's client \
                 certificate is REFUSED until it is one of them: a setting that exists to retire \
                 a credential does not leave it accepted on a misspelling. Sign in with a passkey"
            )),
        ),
    }
}

/// **The setting that overrides how long a client has to finish its TLS
/// handshake** — `ADR-046` §10.
///
/// A whole number of seconds, from [`HANDSHAKE_SECONDS_MIN`] to
/// [`HANDSHAKE_SECONDS_MAX`]. **Absent means [`HANDSHAKE_SECONDS_DEFAULT`]**, so
/// a machine whose document says nothing keeps the deadline it always had. It
/// is the one deadline a person waits inside — a browser pauses the handshake
/// while its user picks a client certificate — which is why a document can change
/// it and no other.
pub const HANDSHAKE_SECONDS_SETTING: &str = "handshake-seconds";

/// [`HANDSHAKE_SECONDS_SETTING`]'s value when the document gives none, or gives
/// one the proxy cannot use.
pub const HANDSHAKE_SECONDS_DEFAULT: u64 = 15;

/// The shortest handshake deadline a document may set: the first-byte and
/// request-head deadlines' own length, below which the handshake would be the
/// strictest step rather than the most patient one.
pub const HANDSHAKE_SECONDS_MIN: u64 = 3;

/// The longest handshake deadline a document may set. Every connection waiting
/// out a handshake holds one of the proxy's connection slots for that long, so
/// an unbounded value would let a few silent clients hold the front door.
pub const HANDSHAKE_SECONDS_MAX: u64 = 120;

/// **Read one [`HANDSHAKE_SECONDS_SETTING`] value**, or say why it is refused.
///
/// A whole number written in decimal digits alone, inside
/// [`HANDSHAKE_SECONDS_MIN`]..=[`HANDSHAKE_SECONDS_MAX`]. Surrounding
/// whitespace is ignored; a sign, a fraction or a unit is not a whole number of
/// seconds and is refused rather than read.
///
/// # Errors
///
/// A sentence naming the value and the accepted range.
pub fn parse_handshake_seconds(value: &str) -> Result<u64, String> {
    let trimmed = value.trim();
    let seconds = if !trimmed.is_empty() && trimmed.bytes().all(|b| b.is_ascii_digit()) {
        trimmed.parse::<u64>().ok()
    } else {
        None
    };
    match seconds {
        Some(s) if (HANDSHAKE_SECONDS_MIN..=HANDSHAKE_SECONDS_MAX).contains(&s) => Ok(s),
        _ => Err(format!(
            "the `{HANDSHAKE_SECONDS_SETTING}` setting is {value:?}, which is not a whole number \
             of seconds from {HANDSHAKE_SECONDS_MIN} to {HANDSHAKE_SECONDS_MAX}"
        )),
    }
}

/// **The handshake deadline the proxy uses, and what to journal about it.**
///
/// Absent is [`HANDSHAKE_SECONDS_DEFAULT`]. **A value [`parse_handshake_seconds`]
/// refuses is also [`HANDSHAKE_SECONDS_DEFAULT`]**, with a sentence saying so:
/// the document's edges refuse such a value before it is written, so one that
/// arrives anyway came by another route, and the front door must still start.
#[must_use]
pub fn handshake_seconds(setting: Option<&str>) -> (u64, Option<String>) {
    match setting.map(parse_handshake_seconds) {
        None => (HANDSHAKE_SECONDS_DEFAULT, None),
        Some(Ok(seconds)) => (seconds, None),
        Some(Err(why)) => (
            HANDSHAKE_SECONDS_DEFAULT,
            Some(format!(
                "{why}. The handshake deadline is {HANDSHAKE_SECONDS_DEFAULT}s, the default, \
                 until the document gives a value in that range"
            )),
        ),
    }
}

/// The file, inside the directory `RIVET_APP_DATA` names for the proxy.
///
/// Named for what it is rather than for the daemon, because the daemon is not
/// the only thing that may ever read it.
pub const FILE: &str = "routes";

/// `<volume>/app/<name>/routes` — **root-owned, application-readable.**
///
/// **Beside `current` rather than inside `data/`, and that is
/// [`crate::settings::path`]'s move applied to a file that had not been written
/// yet.** `data/` is the one directory the application owns, at mode `0700`
/// under its own uid; writing into it as root by path is the shape that
/// produced the 2026-09-03 audit's symlink truncation. `<volume>/app/<name>` is
/// root-owned at `0755`, created by the installer, so there is nowhere for an
/// unprivileged program to plant a link and the class is gone rather than
/// guarded.
///
/// `ADR-046` §5 calls this *"the same arrangement as `ADR-038`'s settings
/// file, deliberately"*, and `ADR-038` §8.1 is where that arrangement now is.
#[must_use]
pub fn path(volume: &str, name: &crate::service::ServiceName) -> String {
    format!("{}/{FILE}", crate::service::app_dir(volume, name))
}

/// The mode the table is written with — `0644`, root-owned.
///
/// The same as [`crate::settings::MODE`] and for the same reason: the proxy
/// runs as its own uid and a table it cannot open is a machine that reports
/// delivery and serves nothing. `ADR-046` §6's *"the list of what a machine
/// runs is not public"* is a statement about the **network**, which is what the
/// gate answers; it was never a claim that one uid on the machine is kept from
/// reading a list of ports that `get status` prints anyway.
pub const MODE: u32 = 0o644;

/// One application, reachable at one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    /// The single path segment, from the manifest's `proxy-path`.
    pub path: String,
    /// The application this belongs to, for the journal and for refusals.
    pub service: String,
    /// The loopback port its first TCP `listen` declared.
    pub port: u16,
    /// **Whether the proxy refuses a caller with no operator certificate** —
    /// `ADR-046` §4.3, from the manifest's `proxy-auth`.
    ///
    /// `true` for every route that did not say otherwise, including every route
    /// written by a build that predates the column — see [`parse`].
    ///
    /// **This is the route's ROOT answer**, and [`Route::requires_operator_for`]
    /// is the one a gate asks: a prefix under this route may say something else
    /// (`ADR-048` §4).
    pub requires_operator: bool,
    /// **What individual prefixes under this route ask** — `ADR-048` §4, from
    /// the manifest's `proxy-auth-paths`.
    ///
    /// Empty is the ordinary answer and means the whole route takes
    /// [`Self::requires_operator`]. Held sorted by prefix and deduplicated,
    /// because a prefix carrying two verdicts would make the answer depend on
    /// which row a reader reached first.
    pub rules: Vec<crate::package::application::ProxyRule>,
}

impl Route {
    /// **Whether the proxy must refuse a caller with no operator certificate,
    /// for this particular request** — `ADR-048` §4.
    ///
    /// `rest` is what [`route_for`] returned: the request path with this
    /// route's own prefix already removed, always beginning with `/`.
    ///
    /// # Longest match wins, and matching is on WHOLE SEGMENTS
    ///
    /// A rule at `pair` answers for `/pair` and `/pair/anything`. It does **not**
    /// answer for `/pairings`, `/pair-admin`, `/Pair` or `/%70air`, none of
    /// which is the same sequence of segments — and a `starts_with` on the raw
    /// string would have opened the first two. That is the failure this whole
    /// mechanism turns on: it is invisible once written, and the service it
    /// would sit on holds sensitive personal data.
    ///
    /// The deepest matching rule answers, so a gated `pair/admin` beats an open
    /// `pair` however the two were written down. **Nothing here reads an order**
    /// — not the manifest's, not the table's — because a rule whose result
    /// depends on declaration order is a rule nobody can audit.
    ///
    /// Where no rule matches, the route's own [`Self::requires_operator`] is the
    /// answer, which is what makes the whole-application form of the
    /// declaration keep working unchanged.
    ///
    /// **It never decodes and it never normalises.** `%70air` is not `pair`
    /// here, so an encoded spelling of an open prefix is *gated* rather than
    /// opened — the direction that costs a refused request rather than the
    /// machine. Dot segments are refused before this is reached, by
    /// [`is_traversal_free`], because `/pair/../admin` would otherwise match an
    /// open rule at `pair` and arrive at a backend that normalises it.
    #[must_use]
    pub fn requires_operator_for(&self, rest: &str) -> bool {
        requires_operator_under(self.requires_operator, &self.rules, rest)
    }

    /// The prefixes under this route that are served without an operator
    /// certificate — for a journal line and for the index, never for a decision.
    #[must_use]
    pub fn open_prefixes(&self) -> Vec<&str> {
        self.rules
            .iter()
            .filter(|rule| !rule.auth().requires_operator())
            .map(crate::package::application::ProxyRule::prefix)
            .collect()
    }

    /// **Whether this route carries at least one `plain:` rule** — `ADR-061`
    /// §7.1. The table-wide question `is_plain_route` answers over the whole
    /// table; this is the one row's half of it.
    #[must_use]
    pub fn has_plain_rule(&self) -> bool {
        self.rules
            .iter()
            .any(|rule| rule.auth() == crate::package::application::ProxyAuth::Plain)
    }

    /// **The prefixes under this route served over plain HTTP** — `ADR-061`
    /// §7.1, for a journal line and for the index, never for a decision.
    /// [`Self::open_prefixes`]'s reason, narrowed to the one verdict that also
    /// opens port 80.
    #[must_use]
    pub fn plain_prefixes(&self) -> Vec<&str> {
        self.rules
            .iter()
            .filter(|rule| rule.auth() == crate::package::application::ProxyAuth::Plain)
            .map(crate::package::application::ProxyRule::prefix)
            .collect()
    }

    /// **Whether `rest`'s longest-matching rule is `plain:`** — `ADR-061` §7.
    ///
    /// The same longest-match [`Self::requires_operator_for`] runs, asking a
    /// different question of the same winning rule: a request the supervisor
    /// bound port 80 for is served there only when the deepest matching rule
    /// under this route is `plain:`. A route with no matching rule, or whose
    /// matching rule is anything else, answers `false` — there is no "plain by
    /// default", because a whole route can never itself be `plain:`, the bare
    /// `proxy-auth` key's grammar being `operator` and `none` only (`ADR-046`
    /// §4).
    #[must_use]
    pub fn is_plain_for(&self, rest: &str) -> bool {
        is_plain_under(&self.rules, rest)
    }
}

/// **Whether any route in the table carries a `plain:` rule** — `ADR-061`
/// §7.1, `.rulings/` R-221. This is the question `cmd/rivet-sysd` asks to
/// decide whether the proxy needs port 80 bound at all: a machine with no
/// answer here has no listener on 80, full stop.
#[must_use]
pub fn any_plain_rule(routes: &[Route]) -> bool {
    routes.iter().any(Route::has_plain_rule)
}

/// **The longest-match itself** — [`Route::requires_operator_for`]'s whole
/// answer, over a route's root verdict and its rules, with no route around it.
///
/// It is a free function so that the manifest can ask the question a live
/// request will ask, before the artifact is signed:
/// `Manifest::requires_operator_at` calls this with the manifest's own
/// `proxy-auth` and `proxy-auth-paths`, and `ADR-051` §12 refuses a restore
/// route that answers `false`. One computation, two callers (`ADR-011`) —
/// a second copy in the manifest would be a rule about opening a door that
/// could drift from the rule the door actually runs.
///
/// `rest` may begin with `/` or not; everything else is
/// [`Route::requires_operator_for`]'s documentation.
#[must_use]
pub fn requires_operator_under(
    route_requires_operator: bool,
    rules: &[crate::package::application::ProxyRule],
    rest: &str,
) -> bool {
    longest_match(rules, rest).map_or(route_requires_operator, |rule| {
        rule.auth().requires_operator()
    })
}

/// **Whether `rest`'s longest-matching rule is `plain:`** — [`Route::is_plain_for`]'s
/// whole answer, over a route's rules alone, with no route around it. A free
/// function for [`any_plain_rule`]'s reason: the same computation is asked
/// both of a live route and, one day, of a manifest before it is signed.
///
/// There is no root case to fall back to, unlike [`requires_operator_under`]:
/// a whole route can never itself be `plain:` (`ADR-046` §4), so no match
/// simply means *not plain*.
#[must_use]
pub fn is_plain_under(rules: &[crate::package::application::ProxyRule], rest: &str) -> bool {
    longest_match(rules, rest)
        .is_some_and(|rule| rule.auth() == crate::package::application::ProxyAuth::Plain)
}

/// **The longest-matching rule itself**, shared by [`requires_operator_under`]
/// and [`is_plain_under`] so the two questions can never disagree about which
/// rule won.
fn longest_match<'a>(
    rules: &'a [crate::package::application::ProxyRule],
    rest: &str,
) -> Option<&'a crate::package::application::ProxyRule> {
    let path = rest.strip_prefix('/').unwrap_or(rest);
    let mut best: Option<&crate::package::application::ProxyRule> = None;
    for rule in rules {
        if !covers(rule.prefix(), path) {
            continue;
        }
        match best {
            Some(deeper) if deeper.depth() >= rule.depth() => {}
            _ => best = Some(rule),
        }
    }
    best
}

/// Whether `path`'s leading segments are exactly `prefix`'s.
///
/// Both are slash-separated and neither is decoded or normalised. An empty
/// `path` (the route's own root) is covered by no prefix, because a prefix has
/// at least one segment and the root has none.
fn covers(prefix: &str, path: &str) -> bool {
    let mut wanted = prefix.split('/');
    let mut got = path.split('/');
    loop {
        match (wanted.next(), got.next()) {
            (None, _) => return true,
            (Some(_), None) => return false,
            (Some(a), Some(b)) => {
                if a != b {
                    return false;
                }
            }
        }
    }
}

/// **Whether a request path can be matched at all without normalising it** —
/// `ADR-048` §5.
///
/// A path is refused when any of its segments is, or percent-decodes to, `.` or
/// `..`, or when a segment holds an encoded separator. Everything else is
/// admitted unchanged.
///
/// # Why the proxy refuses rather than normalising
///
/// `/reliquary/pair/../admin` matches an open rule at `pair` segment by
/// segment, and a backend that resolves the dot segment serves `/admin`. The
/// two ends would disagree about which path was authorised, and the gate's
/// answer would be the wrong one. A proxy that normalised first would instead
/// have to agree with every backend about a normalisation nobody wrote down.
///
/// **`%2f` is refused for the same reason in the other direction**: a segment
/// holding an encoded slash is one segment here and two to anything that
/// decodes, so the matcher and the server would be counting differently.
///
/// This is a property of the request rather than of the routing table, so it
/// binds gated and open routes alike — a traversal that a gate happens to
/// refuse today would otherwise become live the day that route relaxed.
#[must_use]
pub fn is_traversal_free(request_path: &str) -> bool {
    for segment in request_path.split('/') {
        let decoded = percent_decode(segment);
        if decoded == b"." || decoded == b".." || decoded.contains(&b'/') || decoded.contains(&b'\\')
        {
            return false;
        }
    }
    true
}

/// Percent-decode one segment, leaving anything that is not a complete escape
/// exactly as it was.
///
/// **For [`is_traversal_free`]'s question alone.** Nothing forwards these bytes
/// and nothing matches on them; the proxy passes the caller's spelling through
/// untouched, which is what keeps it out of the business of agreeing with every
/// backend about normalisation.
fn percent_decode(segment: &str) -> Vec<u8> {
    fn nibble(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%'
            && at + 2 < bytes.len()
            && let (Some(high), Some(low)) = (nibble(bytes[at + 1]), nibble(bytes[at + 2]))
        {
            out.push(high * 16 + low);
            at += 3;
            continue;
        }
        out.push(bytes[at]);
        at += 1;
    }
    out
}

/// A path two applications both asked for.
///
/// **Reported rather than resolved.** `ADR-015` §6.1 requires an unenforceable
/// declaration to be *reported*, and this is the same rule pointed at a
/// conflict: an operator who sees *"aura-controller and aura-renderer both ask
/// for /aura"* can uninstall one, and an operator whose application silently
/// lost a race cannot even tell that is what happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collision {
    pub path: String,
    /// Every application that asked, in the order they were considered.
    pub claimants: Vec<String>,
}

/// What one installed application asked for, as [`build`] takes it.
///
/// **A struct rather than a tuple since 2026-09-05**, when `ADR-046` §4.3 added
/// a fourth column: three positional strings-and-a-number was already at the
/// edge of readable, and the field this added is a `bool` whose two values are
/// *gated* and *public*. A caller that passed it in the wrong position would
/// compile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    /// The installed application's service name.
    pub service: String,
    /// Its manifest's `proxy-path`.
    pub path: String,
    /// Its first TCP `listen` port.
    pub port: u16,
    /// Its manifest's `proxy-auth`, as
    /// [`crate::package::application::ProxyAuth::requires_operator`] answers it.
    pub requires_operator: bool,
    /// Its manifest's `proxy-auth-paths` — `ADR-048` §4. Empty is the ordinary
    /// answer.
    pub rules: Vec<crate::package::application::ProxyRule>,
}

/// Build the table from what each application declared.
///
/// `wanted` is one [`Wanted`] for every installed application that asked for a
/// path. Applications that asked for none are not passed in — absence means
/// *not proxied*, and a caller that filtered correctly cannot express *asked
/// for nothing* here.
///
/// Returns the routes that are unambiguous, sorted by path, and every collision
/// separately. **A collision removes ALL its claimants**, including the first:
/// serving whichever installed earliest would make reachability depend on
/// install order, which is the property this refuses to have.
#[must_use]
pub fn build(wanted: &[Wanted]) -> (Vec<Route>, Vec<Collision>) {
    let mut sorted: Vec<&Wanted> = wanted.iter().collect();
    // By path, then by service, so the output is a function of the input set
    // rather than of the order a directory happened to be read in.
    sorted.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.service.cmp(&b.service)));

    let mut routes = Vec::new();
    let mut collisions = Vec::new();
    let mut index = 0;
    while index < sorted.len() {
        let path = &sorted[index].path;
        let mut end = index;
        while end < sorted.len() && &sorted[end].path == path {
            end += 1;
        }
        if end - index == 1 {
            let one = sorted[index];
            routes.push(Route {
                path: one.path.clone(),
                service: one.service.clone(),
                port: one.port,
                requires_operator: one.requires_operator,
                rules: one.rules.clone(),
            });
        } else {
            collisions.push(Collision {
                path: path.clone(),
                claimants: sorted[index..end]
                    .iter()
                    .map(|w| w.service.clone())
                    .collect(),
            });
        }
        index = end;
    }

    (routes, collisions)
}

/// The word in the fourth column for a route that needs an operator.
pub const GATED: &str = "operator";
/// The word in the fourth column for a route that does not — `ADR-046` §4.3.
pub const PUBLIC: &str = "public";
/// **The word for a per-path rule the application authenticates itself** —
/// `ADR-048` §4, `ADR-061` §7. Never a whole route's fourth column; only a
/// rule field.
pub const APP: &str = "app";
/// **The word for a per-path rule served over plain HTTP on port 80 too** —
/// `ADR-048` §4, `ADR-061` §7. Never a whole route's fourth column; only a
/// rule field.
pub const PLAIN: &str = "plain";

/// **The application behind the administration portal** — `rivet-fleetd`. A
/// fleet server runs it and a member runs none, which is what makes the portal
/// row conditional. Named here, beside the port, so `rivet-sysd`'s check of the
/// installed set and the proxy's row agree on one spelling.
pub const ADMIN_PORTAL_SERVICE: &str = "rivet-fleetd";

/// **`rivet-fleetd`'s administration port** — `ADR-046` §7, `R-152.1`. It moved
/// off 443 so this proxy could have the front door, and keeps its own mTLS
/// here; the proxy reaches it directly rather than routing to it.
pub const ADMIN_PORTAL_PORT: u16 = 4443;

/// The reserved line that carries the administration portal — a comment, so a
/// proxy that predates it skips it (`parse` drops every `#` line) and a
/// `rivet-sysd` that predates it emits none, which is exactly the version skew
/// [`render`]'s own note describes.
const ADMIN_MARKER: &str = "#admin ";

/// **The administration portal a machine offers beside the proxy** — the
/// `rivet-fleetd` row at [`ADMIN_PORTAL_PORT`].
///
/// It is `Some` **only when that application is installed on the machine**, and
/// that is the whole point: a fleet SERVER runs `rivet-fleetd` and a MEMBER runs
/// none, so a member's page must offer no portal. `rivet-fleetd` declares no
/// `proxy-path` — it is reached directly, not routed (`ADR-046` §7) — so it is
/// not a [`Route`], and this is how its presence rides in the same table the
/// footer says the page is *derived from installed manifests*. Before this, the
/// proxy rendered the row unconditionally, which asserted a fleet server on a
/// machine that had none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminPortal {
    /// The service behind it — `rivet-fleetd`, named rather than assumed so the
    /// row's label is derived like every other.
    pub service: String,
    /// The port it answers on, [`ADMIN_PORTAL_PORT`].
    pub port: u16,
}

/// The administration portal named in a rendered table, or `None`.
///
/// **Read separately from [`parse`]**, because the portal is not a route and
/// [`parse`] is deliberately Routes-only. A table with no `#admin` line — one
/// written by a `rivet-sysd` that predates this, or by any machine with no
/// `rivet-fleetd` installed — answers `None`, which is the honest default: no
/// portal is offered.
#[must_use]
pub fn admin_portal(text: &str) -> Option<AdminPortal> {
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix(ADMIN_MARKER) else {
            continue;
        };
        let mut fields = rest.split_whitespace();
        let (Some(service), Some(port)) = (fields.next(), fields.next()) else {
            continue;
        };
        let Ok(port) = port.parse::<u16>() else {
            continue;
        };
        if port == 0 {
            continue;
        }
        return Some(AdminPortal {
            service: service.to_string(),
            port,
        });
    }
    None
}

/// The table, as the proxy reads it.
///
/// One route per line, `<path> <service> <port> <operator|public>` and then one
/// field per per-path rule, `<operator|public>:<prefix>` — which is the same
/// shape and the same reasoning as [`crate::settings`]'s file: `rivet-core` has
/// an empty `[dependencies]` table by rule (`ADR-014` §3), so there is no TOML
/// parser here and the format is as narrow as the emitter.
///
/// **The gate is a WORD and not a flag**, in the fourth column and in every rule
/// after it, because the file is read by people as well as by the proxy, and
/// `reliquary reliquary-rivet 8443 true` does not say what is true.
///
/// # A route carrying rules is invisible to a proxy that predates them
///
/// `rivet-sysd` writes this file and ships inside the image; `rivet-proxyd`
/// reads it and ships as an application, so the two versions skew (`ADR-048`
/// §6). A proxy built before `ADR-048` drops any line with a fifth field
/// outright, so a route with per-path rules simply is not served — every
/// request under it is `404` — rather than being served at its root gate with
/// the rules silently missing. **That is the fail-closed half of the skew** and
/// it is deliberate: the alternative reading would serve an application's whole
/// surface at whichever verdict the root happened to carry.
#[must_use]
pub fn render(routes: &[Route], admin_portal: Option<&AdminPortal>) -> String {
    let mut out = String::new();
    for route in routes {
        out.push_str(&route.path);
        out.push(' ');
        out.push_str(&route.service);
        out.push(' ');
        out.push_str(&route.port.to_string());
        out.push(' ');
        out.push_str(if route.requires_operator { GATED } else { PUBLIC });
        for rule in &route.rules {
            out.push(' ');
            out.push_str(rule_word(rule.auth()));
            out.push(':');
            out.push_str(rule.prefix());
        }
        out.push('\n');
    }
    // **The administration portal, last and as a `#` comment**, so that only a
    // proxy that knows to look for it reads it and every other reader skips it.
    // Emitted only when there IS one — a member offers none — which is what
    // makes the proxy's row conditional rather than asserted.
    if let Some(portal) = admin_portal {
        out.push_str(ADMIN_MARKER);
        out.push_str(&portal.service);
        out.push(' ');
        out.push_str(&portal.port.to_string());
        out.push('\n');
    }
    out
}

/// Read the table back.
///
/// **A malformed line is skipped, not fatal.** `ADR-036` §8.5.2's rule — an
/// application treats its own unparseable state as absent and cold-starts —
/// and it matters more here than usual: a proxy that refused to start over one
/// bad row would take every OTHER application off the network with it.
///
/// A path that would not satisfy
/// [`crate::package::application::is_mountable_proxy_path`] is dropped, so a
/// table written by something other than [`render`] cannot introduce a route
/// that escapes its own prefix or claims the machine's own sign-in path.
///
/// # The fourth column may be absent, and its absence is GATED
///
/// `ADR-046` §4.3 added it on 2026-09-05, and `rivet-proxyd` is an application
/// rather than image machinery — so a supervisor that predates the column and a
/// proxy that expects one can be on the same board at once. A three-field line
/// is therefore read as a route that **requires an operator**, which is the
/// same direction the manifest key leans and for the same reason: the version
/// skew that fails closed costs a refused request, and the one that fails open
/// costs the machine.
///
/// A fourth field that is neither [`GATED`] nor [`PUBLIC`] drops the line
/// entirely rather than guessing, because guessing here has one safe answer and
/// one that reads *serve this to anybody*.
///
/// # The fields after the fourth are per-path rules — `ADR-048` §4
///
/// Each is `<operator|public>:<prefix>`, and **any one of them that this build
/// cannot read drops the whole line**, including a set that is not sorted by
/// prefix or that names one prefix twice. The row is a security declaration
/// read from a file; a partially-understood one would be a gate with a hole in
/// it, and a route that is absent is a `404` rather than an open door.
#[must_use]
pub fn parse(text: &str) -> Vec<Route> {
    let mut out = Vec::new();
    'line: for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split(' ').filter(|f| !f.is_empty());
        let (Some(path), Some(service), Some(port), gate) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            continue;
        };
        if !crate::package::application::is_mountable_proxy_path(path) {
            continue;
        }
        let Ok(port) = port.parse::<u16>() else {
            continue;
        };
        if port == 0 {
            continue;
        }
        let requires_operator = match gate {
            None | Some(GATED) => true,
            Some(PUBLIC) => false,
            Some(_) => continue,
        };
        let mut rules = Vec::new();
        for field in fields {
            let Some(rule) = parse_rule(field) else {
                continue 'line;
            };
            rules.push(rule);
        }
        // The emitter writes them sorted and never writes one prefix twice, so
        // anything else is a second producer — and a prefix with two verdicts
        // would make the gate's answer depend on which field came first.
        let mut canonical = rules.clone();
        canonical.sort_by(|a, b| a.prefix().cmp(b.prefix()));
        canonical.dedup_by(|a, b| a.prefix() == b.prefix());
        if canonical != rules {
            continue;
        }
        out.push(Route {
            path: path.to_string(),
            service: service.to_string(),
            port,
            requires_operator,
            rules,
        });
    }
    out
}

/// One `<operator|public>:<prefix>` field.
///
/// **The table's two words, not the manifest's** — [`GATED`] and [`PUBLIC`]
/// rather than `operator` and `none`. The fourth column has said `public` since
/// `ADR-046` §4 because a person reads this file and *none* in a column does not
/// say *none of what*; a rule beside it spelling the same verdict differently
/// would be worse than either. The manifest keeps its own spelling for the same
/// reason, and [`crate::package::application::ProxyRule`] is the one type both
/// produce.
fn parse_rule(field: &str) -> Option<crate::package::application::ProxyRule> {
    let (word, prefix) = field.split_once(':')?;
    let auth = match word {
        GATED => crate::package::application::ProxyAuth::Operator,
        PUBLIC => crate::package::application::ProxyAuth::None,
        APP => crate::package::application::ProxyAuth::App,
        PLAIN => crate::package::application::ProxyAuth::Plain,
        _ => return None,
    };
    crate::package::application::ProxyRule::new(auth, prefix)
}

/// The table's own word for a rule's verdict — [`GATED`], [`PUBLIC`], [`APP`]
/// or [`PLAIN`]. Never the manifest's spelling; see [`parse_rule`]'s doc
/// comment for why the table keeps its own vocabulary.
const fn rule_word(auth: crate::package::application::ProxyAuth) -> &'static str {
    match auth {
        crate::package::application::ProxyAuth::Operator => GATED,
        crate::package::application::ProxyAuth::None => PUBLIC,
        crate::package::application::ProxyAuth::App => APP,
        crate::package::application::ProxyAuth::Plain => PLAIN,
    }
}

/// The route a request path belongs to, and what is left of the path.
///
/// `/reliquary/health` against a route at `reliquary` gives `("/health")`.
/// `/reliquary` with nothing after it gives `("/")`, because an application
/// mounted at a prefix is serving its own root there.
///
/// **Matching is on whole segments.** `/reliquary-backup/x` does NOT match a
/// route at `reliquary`, which a `starts_with` on the raw string would have
/// got wrong — and that is the failure that turns a proxy into somebody else's
/// front door.
#[must_use]
pub fn route_for<'a>(routes: &'a [Route], request_path: &str) -> Option<(&'a Route, String)> {
    let rest = request_path.strip_prefix('/')?;
    let (head, tail) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, ""),
    };
    let route = routes.iter().find(|r| r.path == head)?;
    let remainder = if tail.is_empty() {
        String::from("/")
    } else {
        tail.to_string()
    };
    Some((route, remainder))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **Absent is accepted, so no existing machine changes**; `refuse` turns
    /// the certificate off; anything else is refused and says why, because a
    /// misspelling must not leave a retired credential accepted.
    #[test]
    fn the_operator_certificate_is_accepted_unless_a_document_says_otherwise() {
        assert_eq!(operator_certificate(None), (OperatorCertificate::Accepted, None));
        assert_eq!(
            operator_certificate(Some(CERTIFICATE_ACCEPTED)),
            (OperatorCertificate::Accepted, None)
        );
        assert_eq!(
            operator_certificate(Some(CERTIFICATE_REFUSED)),
            (OperatorCertificate::Refused, None)
        );
        let (verdict, said) = operator_certificate(Some("no"));
        assert_eq!(verdict, OperatorCertificate::Refused);
        let said = said.expect("a misspelling is journalled");
        assert!(said.contains("\"no\""), "{said}");
        assert!(said.contains("passkey"), "{said}");
    }

    /// The setting is one the image delivers to the proxy, so a document
    /// naming it is not refused as undeclared.
    #[test]
    fn the_proxy_is_handed_the_operator_certificate_setting() {
        let home = crate::service::image_service_home(crate::service::PROXYD_SERVICE_NAME)
            .expect("the proxy has a home");
        assert!(home.settings.contains(&OPERATOR_CERTIFICATE_SETTING));
        assert!(home.settings.contains(&UPSTREAM_NAME_SETTING));
        assert!(!crate::package::application::is_credential_like_name(OPERATOR_CERTIFICATE_SETTING));
        assert!(home.settings.contains(&HANDSHAKE_SECONDS_SETTING));
        assert!(!crate::package::application::is_credential_like_name(HANDSHAKE_SECONDS_SETTING));
    }

    /// **Absent is fifteen seconds, so no existing machine changes.**
    #[test]
    fn the_handshake_deadline_is_fifteen_seconds_when_the_document_says_nothing() {
        assert_eq!(handshake_seconds(None), (15, None));
        assert_eq!(HANDSHAKE_SECONDS_DEFAULT, 15);
    }

    /// A value in range is the deadline, at both ends of the range.
    #[test]
    fn a_handshake_deadline_in_range_is_used() {
        assert_eq!(handshake_seconds(Some("30")), (30, None));
        assert_eq!(handshake_seconds(Some("3")), (3, None));
        assert_eq!(handshake_seconds(Some("120")), (120, None));
        assert_eq!(handshake_seconds(Some(" 45 ")), (45, None));
        assert_eq!(parse_handshake_seconds("30"), Ok(30));
    }

    /// **Out of range, or not a whole number, is refused naming the range**,
    /// which is the sentence the document's edges give an operator.
    #[test]
    fn a_handshake_deadline_out_of_range_or_not_whole_is_refused_naming_the_range() {
        for bad in ["2", "121", "500", "0", "", "fifteen", "15s", "15.0", "-5", "+30", "1e2"] {
            let why = parse_handshake_seconds(bad).expect_err(bad);
            assert!(why.contains("3 to 120"), "{bad:?}: {why}");
            assert!(why.contains(HANDSHAKE_SECONDS_SETTING), "{bad:?}: {why}");
        }
    }

    /// **A bad value that reaches the proxy anyway is fifteen seconds and a
    /// sentence**, never a refusal to start.
    #[test]
    fn a_bad_handshake_deadline_delivered_anyway_falls_back_to_fifteen_and_says_so() {
        let (seconds, said) = handshake_seconds(Some("500"));
        assert_eq!(seconds, 15);
        let said = said.expect("a bad value is journalled");
        assert!(said.contains("\"500\""), "{said}");
        assert!(said.contains("15s"), "{said}");
        let (seconds, said) = handshake_seconds(Some("soon"));
        assert_eq!(seconds, 15);
        assert!(said.expect("journalled").contains("\"soon\""));
    }

    /// A gated route — what a manifest that says nothing about callers asks
    /// for.
    fn want(service: &str, path: &str, port: u16) -> Wanted {
        Wanted {
            service: service.to_string(),
            path: path.to_string(),
            port,
            requires_operator: true,
            rules: Vec::new(),
        }
    }

    /// One that relaxed — `ADR-046` §4.3, `R-152`'s TempMon.
    fn want_public(service: &str, path: &str, port: u16) -> Wanted {
        Wanted {
            requires_operator: false,
            ..want(service, path, port)
        }
    }

    /// `ADR-048` §4's per-path rules, written as the manifest spells them.
    fn rule(text: &str) -> crate::package::application::ProxyRule {
        crate::package::application::ProxyRule::parse(text).expect("a rule this test meant to write")
    }

    /// A gated application with per-path rules, sorted as the emitter sorts.
    fn want_with(service: &str, path: &str, port: u16, rules: &[&str]) -> Wanted {
        let mut rules: Vec<_> = rules.iter().map(|text| rule(text)).collect();
        rules.sort_by(|a, b| a.prefix().cmp(b.prefix()));
        Wanted {
            rules,
            ..want(service, path, port)
        }
    }

    /// The single route `want_with` describes, as the gate sees it.
    fn one(rules: &[&str]) -> Route {
        let (routes, _) = build(&[want_with("reliquary-rivet", "reliquary", 8443, rules)]);
        routes.into_iter().next().expect("one route")
    }

    #[test]
    fn an_unambiguous_set_becomes_routes_sorted_by_path() {
        let (routes, collisions) = build(&[
            want("trends", "trends", 8444),
            want("reliquary-rivet", "reliquary", 8443),
        ]);
        assert!(collisions.is_empty());
        assert_eq!(
            routes.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
            ["reliquary", "trends"]
        );
    }

    /// **Both claimants lose.** Serving whichever installed first would make
    /// reachability depend on install order, and an operator whose application
    /// silently lost a race cannot tell that is what happened.
    #[test]
    fn a_contested_path_removes_every_claimant_and_is_reported() {
        let (routes, collisions) = build(&[
            want("aura-controller", "aura", 8080),
            want("aura-renderer", "aura", 8081),
            want("reliquary-rivet", "reliquary", 8443),
        ]);
        assert_eq!(routes.len(), 1, "only the uncontested route survives");
        assert_eq!(routes[0].service, "reliquary-rivet");
        assert_eq!(collisions.len(), 1);
        assert_eq!(collisions[0].path, "aura");
        assert_eq!(collisions[0].claimants, ["aura-controller", "aura-renderer"]);
    }

    #[test]
    fn the_table_is_a_function_of_the_set_and_not_of_its_order() {
        let a = build(&[
            want("trends", "trends", 8444),
            want("reliquary-rivet", "reliquary", 8443),
        ]);
        let b = build(&[
            want("reliquary-rivet", "reliquary", 8443),
            want("trends", "trends", 8444),
        ]);
        assert_eq!(a, b);
    }

    #[test]
    fn a_rendered_table_parses_back_identically() {
        let (routes, _) = build(&[
            want("reliquary-rivet", "reliquary", 8443),
            want("trends", "trends", 8444),
        ]);
        assert_eq!(parse(&render(&routes, None)), routes);
    }

    /// **The administration portal rides in the table and is not a route.**
    /// When it is present the `#admin` line round-trips through `admin_portal`,
    /// and `parse` still sees only the routes — a member (no portal) renders no
    /// such line and reads back `None`, which is the whole point: the proxy's
    /// `rivet-fleetd` row becomes conditional rather than asserted.
    #[test]
    fn the_admin_portal_rides_beside_the_routes_and_is_absent_for_a_member() {
        let (routes, _) = build(&[want("reliquary-rivet", "reliquary", 8443)]);

        // A fleet server: fleetd is installed, so the portal is rendered.
        let portal = AdminPortal {
            service: "rivet-fleetd".to_string(),
            port: ADMIN_PORTAL_PORT,
        };
        let served = render(&routes, Some(&portal));
        assert_eq!(admin_portal(&served), Some(portal));
        // The directive is not a route — parse ignores it entirely.
        assert_eq!(parse(&served), routes);

        // A member: no fleetd, no line, no portal.
        let member = render(&routes, None);
        assert_eq!(admin_portal(&member), None);
        assert!(!member.contains("#admin"), "a member's table named a portal: {member}");
        assert_eq!(parse(&member), routes);
    }

    /// A proxy that refused to start over one bad row would take every other
    /// application off the network with it.
    #[test]
    fn a_malformed_line_is_skipped_and_the_rest_still_load() {
        let text = "reliquary reliquary-rivet 8443\n\
                    this line is nonsense with too many fields\n\
                    \n\
                    # a comment\n\
                    trends trends 8444\n";
        let routes = parse(text);
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].path, "reliquary");
        assert_eq!(routes[1].path, "trends");
    }

    /// The table could be written by something other than `render`, so the
    /// reader enforces the same shape the manifest does.
    /// **`ADR-046` §4.3's column, through the file.** The word is what a person
    /// reading the table sees, and the round trip is what the proxy depends on.
    #[test]
    fn the_operator_column_round_trips_and_is_a_word() {
        let (routes, _) = build(&[
            want("reliquary-rivet", "reliquary", 8443),
            want_public("tempmon-rivet", "tempmon", 8081),
        ]);
        let text = render(&routes, None);
        assert!(text.contains("reliquary reliquary-rivet 8443 operator\n"), "{text}");
        assert!(text.contains("tempmon tempmon-rivet 8081 public\n"), "{text}");
        assert_eq!(parse(&text), routes);
    }

    /// **The version skew that must fail closed.** `rivet-proxyd` is an
    /// application, so a supervisor predating this column and a proxy expecting
    /// one can be on one board at once. A three-field line is a route that
    /// requires an operator — the skew that costs a refused request rather than
    /// the one that costs the machine.
    #[test]
    fn a_row_written_before_the_column_existed_is_gated() {
        let routes = parse("reliquary reliquary-rivet 8443\n");
        assert_eq!(routes.len(), 1);
        assert!(
            routes[0].requires_operator,
            "an older table must not open a route"
        );
    }

    /// A fourth field this build does not define is a line dropped, not a guess
    /// — because one of the two guesses reads *serve this to anybody*.
    #[test]
    fn an_unknown_gate_word_drops_the_row_rather_than_guessing() {
        for hostile in ["yes", "true", "none", "OPERATOR", "publi", "1"] {
            assert!(
                parse(&format!("reliquary reliquary-rivet 8443 {hostile}\n")).is_empty(),
                "{hostile:?} must not become a route"
            );
        }
    }

    #[test]
    fn a_table_row_that_could_escape_its_prefix_is_dropped_on_read() {
        for hostile in ["../etc x 80", ".. x 80", "/abs x 80", "a/b x 80"] {
            assert!(
                parse(hostile).is_empty(),
                "{hostile:?} must not become a route"
            );
        }
        assert!(parse("ok svc 0").is_empty(), "port 0 is not a port");
        assert!(parse("ok svc 99999").is_empty(), "a port must fit in u16");
    }

    /// **A row claiming the machine's own sign-in path never becomes a route**
    /// — `ADR-046` §4. The table could be written by something other than
    /// `render`, so the reader refuses what the manifest refuses.
    #[test]
    fn a_table_row_claiming_a_reserved_path_is_dropped_on_read() {
        for reserved in crate::package::application::RESERVED_PROXY_PATHS {
            let routes = parse(&format!("{reserved} svc 8443\nok svc 8444\n"));
            assert_eq!(routes.len(), 1, "{reserved:?} must not become a route");
            assert_eq!(routes[0].path, "ok");
        }
    }

    #[test]
    fn a_request_is_split_into_its_route_and_the_rest() {
        let (routes, _) = build(&[want("reliquary-rivet", "reliquary", 8443)]);
        let (route, rest) = route_for(&routes, "/reliquary/api/v1/blobs").expect("matched");
        assert_eq!(route.service, "reliquary-rivet");
        assert_eq!(rest, "/api/v1/blobs");
    }

    /// An application mounted at a prefix serves its own root there.
    #[test]
    fn a_bare_prefix_becomes_the_applications_root() {
        let (routes, _) = build(&[want("reliquary-rivet", "reliquary", 8443)]);
        assert_eq!(route_for(&routes, "/reliquary").expect("matched").1, "/");
        assert_eq!(route_for(&routes, "/reliquary/").expect("matched").1, "/");
    }

    /// **The failure that turns a proxy into somebody else's front door.** A
    /// `starts_with` on the raw string would route `/reliquary-backup` to
    /// `reliquary`; matching whole segments is what stops it.
    #[test]
    fn a_longer_name_sharing_a_prefix_does_not_match() {
        let (routes, _) = build(&[want("reliquary-rivet", "reliquary", 8443)]);
        assert!(route_for(&routes, "/reliquary-backup/secrets").is_none());
        assert!(route_for(&routes, "/reliquaryx").is_none());
    }

    #[test]
    fn an_unrouted_path_and_the_bare_root_match_nothing() {
        let (routes, _) = build(&[want("reliquary-rivet", "reliquary", 8443)]);
        assert!(route_for(&routes, "/").is_none());
        assert!(route_for(&routes, "/nothing/here").is_none());
        assert!(route_for(&routes, "relative").is_none());
    }

    // --- `ADR-048` §4: per-path auth, tested adversarially --------------------
    //
    // **Every test below is an attempt to open something that must stay shut.**
    // The mechanism is one open prefix on a gated application, which is exactly
    // Reliquary's phone door beside its stored data; a hole here is invisible
    // once written and would sit on that service. So the assertions are written
    // as attacks and named for what they refuse, and the two that check the
    // mechanism WORKS are outnumbered on purpose.

    /// The rule has to do its job before refusing things is interesting: the
    /// exact prefix, and everything under it, is served with no operator.
    #[test]
    fn an_open_prefix_opens_itself_and_everything_under_it() {
        let route = one(&["none:pair"]);
        assert!(!route.requires_operator_for("/pair"));
        assert!(!route.requires_operator_for("/pair/"));
        assert!(!route.requires_operator_for("/pair/redeem"));
        assert!(!route.requires_operator_for("/pair/redeem/abc123"));
    }

    /// **The route's own root is not under an open prefix.** `/reliquary/`
    /// is the page holding the records; the door is `/reliquary/pair`.
    #[test]
    fn an_open_prefix_does_not_open_the_route_root() {
        let route = one(&["none:pair"]);
        assert!(route.requires_operator_for("/"));
        assert!(route.requires_operator_for("/blobs"));
    }

    /// **A sibling sharing a character prefix.** A `starts_with` on the raw
    /// string opens this, and nothing about the resulting page looks wrong.
    #[test]
    fn an_open_pair_does_not_open_pairings() {
        let route = one(&["none:pair"]);
        assert!(route.requires_operator_for("/pairings"));
        assert!(route.requires_operator_for("/pairings/"));
        assert!(route.requires_operator_for("/pairings/all"));
    }

    /// **A sibling ending in a hyphen**, which is a legal name segment here and
    /// is what an administrative endpoint beside a public one tends to be
    /// called.
    #[test]
    fn an_open_pair_does_not_open_pair_admin() {
        let route = one(&["none:pair"]);
        assert!(route.requires_operator_for("/pair-admin"));
        assert!(route.requires_operator_for("/pair-admin/revoke"));
    }

    /// **The matcher is case-sensitive, and the prefix grammar is lowercase**,
    /// so an upper-case spelling matches no rule and takes the route's gate. It
    /// falls the safe way, which is what makes case-sensitivity tolerable here
    /// at all.
    #[test]
    fn an_upper_case_spelling_of_an_open_prefix_stays_gated() {
        let route = one(&["none:pair"]);
        assert!(route.requires_operator_for("/Pair"));
        assert!(route.requires_operator_for("/PAIR/redeem"));
    }

    /// **A percent-encoded segment that decodes to the open prefix.** Nothing
    /// here decodes, so `%70air` is not `pair` and the request is gated — the
    /// direction that costs a caller a `403` rather than costing the machine.
    #[test]
    fn a_percent_encoded_spelling_of_an_open_prefix_stays_gated() {
        let route = one(&["none:pair"]);
        assert!(route.requires_operator_for("/%70air"));
        assert!(route.requires_operator_for("/%70air/redeem"));
        assert!(route.requires_operator_for("/pai%72"));
    }

    /// A rule is anchored at the route's own root; it does not match a segment
    /// that merely appears somewhere in the path.
    #[test]
    fn an_open_prefix_does_not_match_deeper_in_the_path() {
        let route = one(&["none:pair"]);
        assert!(route.requires_operator_for("/api/pair"));
        assert!(route.requires_operator_for("/blobs/pair/redeem"));
    }

    /// The same three refusals at depth, because a multi-segment prefix is
    /// compared the same way and a matcher that anchored only the first segment
    /// would pass every test above.
    #[test]
    fn a_deep_open_prefix_refuses_the_same_neighbours() {
        let route = one(&["none:api/v1/pair"]);
        assert!(!route.requires_operator_for("/api/v1/pair/redeem"));
        assert!(route.requires_operator_for("/api/v1/pairings"));
        assert!(route.requires_operator_for("/api/v1/pair-admin"));
        assert!(route.requires_operator_for("/api/v2/pair"));
        assert!(route.requires_operator_for("/api/v1"));
    }

    /// **Longest match wins**, so a gated path under an open prefix is gated.
    #[test]
    fn a_gated_prefix_under_an_open_one_wins_because_it_is_deeper() {
        let route = one(&["none:pair", "operator:pair/admin"]);
        assert!(!route.requires_operator_for("/pair/redeem"));
        assert!(route.requires_operator_for("/pair/admin"));
        assert!(route.requires_operator_for("/pair/admin/revoke"));
        // And the sibling checks still hold under the deeper rule.
        assert!(!route.requires_operator_for("/pair/administrator"));
    }

    /// **A rule whose result depended on declaration order is a rule nobody can
    /// audit**, so the two orders are asserted to give the same answer — not
    /// merely to produce the same table.
    /// **A rule whose result depended on declaration order is a rule nobody can
    /// audit.** The two vectors here are built in opposite orders and by hand,
    /// bypassing the sort every legitimate producer applies — so this asserts
    /// the MATCHER reads no order, rather than asserting that the emitter
    /// sorted.
    #[test]
    fn the_gate_reads_no_order_even_when_the_rules_arrive_in_one() {
        let route = |rules: Vec<crate::package::application::ProxyRule>| Route {
            path: String::from("reliquary"),
            service: String::from("reliquary-rivet"),
            port: 8443,
            requires_operator: true,
            rules,
        };
        let open_first = route(vec![rule("none:pair"), rule("operator:pair/admin")]);
        let gated_first = route(vec![rule("operator:pair/admin"), rule("none:pair")]);
        for path in [
            "/",
            "/pair",
            "/pair/redeem",
            "/pair/admin",
            "/pair/admin/revoke",
            "/pairings",
            "/pair-admin",
        ] {
            assert_eq!(
                open_first.requires_operator_for(path),
                gated_first.requires_operator_for(path),
                "the two orders disagree about {path}"
            );
        }
        assert!(gated_first.requires_operator_for("/pair/admin"));
        assert!(!gated_first.requires_operator_for("/pair/redeem"));
    }

    /// An open application with a gated island — the other direction, which is
    /// safe to get wrong and is therefore the one nobody tests.
    #[test]
    fn a_gated_prefix_under_an_open_application_is_gated() {
        let (routes, _) = build(&[Wanted {
            requires_operator: false,
            rules: vec![rule("operator:admin")],
            ..want("tempmon-rivet", "tempmon", 8081)
        }]);
        let route = &routes[0];
        assert!(!route.requires_operator_for("/readings"));
        assert!(route.requires_operator_for("/admin"));
        assert!(!route.requires_operator_for("/administration"));
    }

    /// A route that declares nothing answers exactly as it did before
    /// `ADR-048`, which is what keeps every manifest already signed meaning
    /// what it meant.
    #[test]
    fn a_route_with_no_rules_answers_with_its_own_gate() {
        let gated = one(&[]);
        assert!(gated.requires_operator_for("/"));
        assert!(gated.requires_operator_for("/pair"));
        let (open, _) = build(&[want_public("tempmon-rivet", "tempmon", 8081)]);
        assert!(!open[0].requires_operator_for("/"));
        assert!(!open[0].requires_operator_for("/anything/at/all"));
    }

    /// **`/pair/../admin` matches an open rule at `pair` segment by segment**,
    /// and a backend that resolves the dot segment then serves `/admin`. The
    /// two ends would disagree about what was authorised, so the request is
    /// refused before any rule is consulted.
    #[test]
    fn a_dot_segment_is_refused_rather_than_normalised() {
        for hostile in [
            "/reliquary/pair/../blobs",
            "/reliquary/pair/%2e%2e/blobs",
            "/reliquary/pair/.%2e/blobs",
            "/reliquary/pair/%2E%2E/blobs",
            "/reliquary/./blobs",
            "/reliquary/pair/..",
        ] {
            assert!(
                !is_traversal_free(hostile),
                "{hostile:?} must not reach the matcher"
            );
        }
        // And the matcher would indeed have been fooled, which is why the
        // refusal is the thing being tested rather than an extra precaution.
        let route = one(&["none:pair"]);
        assert!(!route.requires_operator_for("/pair/../blobs"));
    }

    /// **An encoded separator is one segment here and two to anything that
    /// decodes**, so the matcher and the backend would be counting differently.
    #[test]
    fn an_encoded_separator_is_refused_because_the_two_ends_would_count_differently() {
        assert!(!is_traversal_free("/reliquary/pair%2fadmin"));
        assert!(!is_traversal_free("/reliquary/pair%2Fadmin"));
        assert!(!is_traversal_free("/reliquary/pair%5cadmin"));
    }

    /// Ordinary paths are admitted unchanged — a guard that refused real
    /// requests would be removed by whoever met it next.
    #[test]
    fn an_ordinary_path_is_admitted_by_the_traversal_guard() {
        for ordinary in [
            "/",
            "/reliquary/pair",
            "/reliquary/blobs/2026-09-07T15-35-25.logbak",
            "/reliquary/pair/A1B2-C3D4",
            "/reliquary/a%20name",
            "/reliquary/..well-known",
            "/reliquary/%zz",
        ] {
            assert!(is_traversal_free(ordinary), "{ordinary:?} is an ordinary path");
        }
    }

    /// The rules survive the file, and the words in it are words.
    #[test]
    fn the_per_path_rules_round_trip_through_the_table() {
        let (routes, _) = build(&[want_with(
            "reliquary-rivet",
            "reliquary",
            8443,
            &["none:pair", "operator:pair/admin"],
        )]);
        let text = render(&routes, None);
        assert!(
            text.contains("reliquary reliquary-rivet 8443 operator public:pair operator:pair/admin\n"),
            "{text}"
        );
        assert_eq!(parse(&text), routes);
    }

    /// **A partially-understood security row is a gate with a hole in it.** A
    /// rule this build cannot read takes the whole route away, which is a `404`
    /// rather than an open door.
    #[test]
    fn a_rule_this_build_cannot_read_drops_the_whole_route() {
        for hostile in [
            "reliquary reliquary-rivet 8443 operator none:pair",
            "reliquary reliquary-rivet 8443 operator public:/pair",
            "reliquary reliquary-rivet 8443 operator public:pair/",
            "reliquary reliquary-rivet 8443 operator public:../etc",
            "reliquary reliquary-rivet 8443 operator public:pair//admin",
            "reliquary reliquary-rivet 8443 operator public:Pair",
            "reliquary reliquary-rivet 8443 operator pair",
            "reliquary reliquary-rivet 8443 operator yes:pair",
        ] {
            assert!(
                parse(hostile).is_empty(),
                "{hostile:?} must not become a route"
            );
        }
    }

    // --- `ADR-048` §4, `ADR-061` §7: `app` and `plain` -------------------------

    /// The two new verdicts round-trip through the table with the table's own
    /// words, distinct from every other rule's.
    #[test]
    fn app_and_plain_round_trip_through_the_table_with_their_own_words() {
        let (routes, _) = build(&[want_with(
            "gantry-rivet",
            "gantry",
            8080,
            &["app:api", "plain:install"],
        )]);
        let text = render(&routes, None);
        assert!(
            text.contains("gantry gantry-rivet 8080 operator app:api plain:install\n"),
            "{text}"
        );
        assert_eq!(parse(&text), routes);
    }

    /// A mixed table of all four verdicts still resolves by longest match, and
    /// `app`/`plain` answer `false` to "does this need an operator" exactly as
    /// `none` does.
    #[test]
    fn longest_match_resolves_correctly_among_all_four_verdicts() {
        let route = one(&["none:pub", "app:api", "plain:install", "operator:api/admin"]);
        assert!(!route.requires_operator_for("/pub"));
        assert!(!route.requires_operator_for("/api"));
        assert!(!route.requires_operator_for("/api/anything"));
        assert!(!route.requires_operator_for("/install"));
        assert!(route.requires_operator_for("/api/admin"));
        assert!(route.requires_operator_for("/api/admin/revoke"));
        // Nothing matched: the route's own root gate answers.
        assert!(route.requires_operator_for("/"));
    }

    /// **The table query `cmd/rivet-sysd` asks to decide whether port 80 is
    /// bound at all.** A table with no `plain:` rule anywhere answers `false`;
    /// one route among many carrying one is enough to answer `true`.
    #[test]
    fn any_plain_rule_answers_for_the_whole_table() {
        let (none_plain, _) = build(&[
            want("trends", "trends", 8444),
            want_with("reliquary-rivet", "reliquary", 8443, &["none:pair"]),
        ]);
        assert!(!any_plain_rule(&none_plain));
        assert!(!none_plain[1].has_plain_rule());

        let (with_plain, _) = build(&[
            want("trends", "trends", 8444),
            want_with("gantry-rivet", "gantry", 8080, &["plain:install"]),
        ]);
        assert!(any_plain_rule(&with_plain));
        // Sorted by path: "gantry" precedes "trends".
        assert!(with_plain[0].has_plain_rule());
        assert!(!with_plain[1].has_plain_rule(), "trends carries no plain rule");
    }

    /// **`Route::is_plain_for` is the per-request question `cmd/rivet-proxyd`
    /// asks on port 80** — the same longest-match, asking whether the winning
    /// rule is `plain:` rather than whether it requires an operator.
    #[test]
    fn is_plain_for_follows_the_same_longest_match() {
        let route = one(&["plain:install", "operator:install/admin", "none:health"]);
        assert!(route.is_plain_for("/install"));
        assert!(route.is_plain_for("/install/apk"));
        assert!(!route.is_plain_for("/install/admin"), "the deeper gated rule wins");
        assert!(!route.is_plain_for("/health"), "none is not plain");
        assert!(!route.is_plain_for("/"), "no rule matches the route's own root");
        assert!(!route.is_plain_for("/nothing/here"));
    }

    /// A prefix carrying two verdicts, or a set in an order the emitter never
    /// writes, is a second producer — and the answer would depend on which
    /// field came first.
    #[test]
    fn a_repeated_or_unsorted_rule_set_drops_the_whole_route() {
        assert!(
            parse("reliquary reliquary-rivet 8443 operator public:pair operator:pair").is_empty(),
            "one prefix must not carry two verdicts"
        );
        assert!(
            parse("reliquary reliquary-rivet 8443 operator operator:pair/admin public:pair")
                .is_empty(),
            "the emitter sorts by prefix; anything else is a second producer"
        );
    }
}
