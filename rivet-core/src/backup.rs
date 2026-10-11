//! **What the applications on this machine offer to whoever backs it up** —
//! `ADR-051`, 2026-09-08.
//!
//! `ADR-030` ruled application data **out of scope** for Rivet's own backup —
//! its table says whole `/state` *"remains the operator's"*, and its History
//! carries an entry titled *why application data is excluded*. So TempMon's
//! readings and Reliquary's blobs were covered by nothing and were meant to
//! be. This module is the other half of that ruling: the
//! platform still does not decide what is kept or for how long, but an
//! application can now **offer** its data, and the offer is machine-readable.
//!
//! # It is [`crate::proxy`] a second time, and deliberately so
//!
//! Every row comes from an installed application's **signed manifest** — its
//! `proxy-path`, and its `backup-path`/`backup-format` or its `backup-feed`.
//! Nothing here is configured. An operator cannot add a backup surface, cannot
//! move one and cannot change what one claims to be: doing any of those means
//! shipping a new artifact that somebody signed.
//!
//! The supervisor derives the table once per convergence and writes it
//! **root-owned**, so that a program running as an application's own uid cannot
//! rewrite what it claims to offer.
//!
//! # Where it goes, and why not beside an application
//!
//! [`crate::proxy::path`] puts the routing table in `rivet-proxyd`'s own
//! `<volume>/app/<name>` directory, because `rivet-proxyd` is the reader and it
//! is on this machine. **This table has no reader on the machine at all.** Its
//! consumer is a workstation, over `ADR-035`'s control plane, so there is no
//! application whose directory it belongs beside — and putting it in one would
//! mean picking an application arbitrarily and making the file disappear when
//! that one was uninstalled.
//!
//! So it is [`PATH`](crate::backup::PATH), in `/run/rivet` beside `crate::display::HOURS_PATH` and
//! `crate::generated::OS_VERSION_PATH`, which is where this machine's derived
//! runtime facts live. `/run/rivet` is root-owned, so the property the proxy
//! table's placement was chosen for — an application cannot rewrite its own
//! row — holds here for free.
//!
//! **It is read with `get file`, and no control-plane subject was added.**
//! `ADR-006` §8's `get file` subject already reads a text file off a machine
//! whole, and `cargo rivet screenshot` already does exactly this against a path in
//! `/state`. A `get backups` would have been a second name for one fact
//! (`ADR-011`) whose only advantage was not having to type the path.
//!
//! # Validation stops at the FORMAT, and never at the schema
//!
//! [`validate`](crate::backup::validate) is the whole of what a client checks,
//! and the line is the maintainer's, 2026-09-08 (`.rulings/` R-306): a client
//! asks whether a file is a well-formed database or a well-formed CSV, and that
//! is as far as it needs to go.
//!
//! *Is this a valid SQLite database* cannot rot. *Is this a valid Trends
//! backup* rots at every migration, and a validator that rots is one that
//! starts refusing good backups on the day somebody adds a column.

use crate::package::application::{BackupFormat, BackupSurface};

/// **Where the machine publishes what its applications offer.**
///
/// `/run/rivet` is Rivet's own runtime namespace and this is a derived runtime
/// fact of exactly `crate::display::HOURS_PATH`'s kind — rewritten every
/// convergence, true only of the machine that wrote it, and gone when it stops.
///
/// **Not in `/etc`**, for that constant's reason: `ADR-007` §3 makes `/etc`
/// a compatibility surface for paths a C runtime demands, and no C runtime asks
/// what an application offers to back up.
///
/// **Not under `/state`**, which is the other tempting home. A table derived
/// from installed manifests every convergence has no business surviving a
/// reboot: one that did would name applications that had since been
/// uninstalled, on a machine that had not converged yet, and a client reading
/// it would fetch from routes that no longer exist.
pub const PATH: &str = "/run/rivet/backups";

/// The mode the table is written with — `0644`, root-owned.
///
/// The same as [`crate::proxy::MODE`] and [`crate::settings::MODE`]. It is
/// world-readable because `get file` reads it over the control plane as an
/// operator and because there is nothing in it that `get status` does not
/// already print: the names of installed applications and a route each.
pub const MODE: u32 = 0o644;

/// **The method a restore is sent with** — `ADR-051` §12's contract.
///
/// `POST` and not `PUT`: a restore is a request the application may refuse
/// after reading the body, and what it replaces is the application's whole
/// data rather than the resource at this URL. It is also what TempMon's own
/// restore route already takes.
pub const RESTORE_METHOD: &str = "POST";

/// **The query a restore is sent with** — `ADR-051` §12's contract.
///
/// The operator has already said `confirm` to `cargo rivet restore`; this
/// carries that confirmation to the application, so a handler that guards a
/// destructive write behind an explicit confirmation — TempMon's does — needs
/// no second route for Rivet. A handler that does not guard may ignore it.
pub const RESTORE_QUERY: &str = "confirm=true";

/// **The control plane's backup route** — `ADR-035` §3, `ADR-051` §8.
///
/// `GET /backup/<service>` on port 7777 asks the machine to fetch that
/// application's declared backup over loopback and stream the answer back. One
/// application per request, named by its service name — the key the table is
/// already keyed by — so the route carries nothing the machine has to trust
/// beyond a name it looks up.
///
/// Spelled here, beside the table, because both ends compose it: the
/// workstation that asks and the daemon that answers (`ADR-011`).
pub const CONTROL_PLANE_BACKUP: &str = "/backup/";

/// **The control plane's restore route** — `ADR-035` §3, `ADR-051` §12.
///
/// `POST /restore/<service>` with a `Content-Length` body: the machine sends
/// the body to that application's declared `restore-path` over loopback and
/// streams the application's answer back.
pub const CONTROL_PLANE_RESTORE: &str = "/restore/";

/// **The header the control plane sets on an answer it composed itself**,
/// rather than one an application sent.
///
/// A relayed answer is the application's bytes, untouched, so a `404` from the
/// application and a `404` from the machine — *that application declares
/// nothing here* — would
/// otherwise read the same. This header is on every refusal the machine makes
/// on these two routes and on nothing it relays.
pub const ANSWERED_BY_HEADER: &str = "Rivet-Answered-By";

/// [`CONTROL_PLANE_BACKUP`] for one application.
#[must_use]
pub fn backup_route(service: &str) -> String {
    format!("{CONTROL_PLANE_BACKUP}{service}")
}

/// [`CONTROL_PLANE_RESTORE`] for one application.
#[must_use]
pub fn restore_route(service: &str) -> String {
    format!("{CONTROL_PLANE_RESTORE}{service}")
}

/// Which of the two routes a request path names, and the service it names —
/// `None` for anything else, including a name that is not a service name.
///
/// The name is checked against the same grammar every real service name
/// satisfies, so nothing past this point is handed a path separator, a dot
/// segment or a query.
#[must_use]
pub fn requested(path: &str) -> Option<(Requested, &str)> {
    let (which, rest) = match path.strip_prefix(CONTROL_PLANE_BACKUP) {
        Some(rest) => (Requested::Backup, rest),
        None => (Requested::Restore, path.strip_prefix(CONTROL_PLANE_RESTORE)?),
    };
    crate::service::ServiceName::new(rest)?;
    Some((which, rest))
}

/// Which control-plane route a request names — [`requested`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requested {
    /// [`CONTROL_PLANE_BACKUP`].
    Backup,
    /// [`CONTROL_PLANE_RESTORE`].
    Restore,
}

/// The word in the third column for the one-fetch shape.
pub const ARTIFACT: &str = "artifact";
/// The word in the third column for the resumable one.
pub const FEED: &str = "feed";
/// The word in the third column for an artifact's restore route —
/// `ADR-051` §12.
///
/// **A line of its own, and not a sixth field on the artifact's line.** A
/// reader built before `restore-path` existed drops a line with a trailing
/// field *whole* — see [`parse`] — so a sixth field would make every older
/// `cargo rivet backup` stop seeing an application the moment it accepted a
/// restore. An unknown shape word drops only its own line, so an older reader
/// keeps the backup and ignores the restore, which is the direction that
/// costs nothing.
pub const RESTORE: &str = "restore";

/// One application's offer, as a client reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// The application this belongs to — its service name.
    pub service: String,
    /// Where `rivet-proxyd` mounts it, from the manifest's `proxy-path`.
    pub route: String,
    /// The surface it declared, relative to that route.
    pub surface: BackupSurface,
}

impl Declaration {
    /// **The path a client requests**, `/<route>/<path>`, with the leading
    /// slash and no host.
    ///
    /// Composed here rather than by every client, for `ADR-011`'s reason: the
    /// relationship between `proxy-path` and `backup-path` is one fact, and a
    /// client that joined the two itself would be a second place to get the
    /// slash wrong.
    #[must_use]
    pub fn request_path(&self) -> String {
        format!("/{}/{}", self.route, self.surface.path())
    }

    /// **The path a restore is sent to**, `/<route>/<restore-path>`, or `None`
    /// when the application accepts no restore — `ADR-051` §12. Composed here
    /// for [`Self::request_path`]'s reason.
    #[must_use]
    pub fn restore_request_path(&self) -> Option<String> {
        self.surface
            .restore_path()
            .map(|path| format!("/{}/{path}", self.route))
    }

    /// **The path the application's own server is asked for** — `/<path>`,
    /// with the route's prefix removed exactly as `rivet-proxyd` removes it
    /// before forwarding (`crate::proxy::route_for`). The machine reaches the
    /// application on loopback with this, so the application sees the same
    /// request line whether a backup came through the proxy or the control
    /// plane.
    #[must_use]
    pub fn application_path(&self) -> String {
        format!("/{}", self.surface.path())
    }

    /// [`Self::application_path`] for the restore route, or `None` when the
    /// application accepts no restore.
    #[must_use]
    pub fn application_restore_path(&self) -> Option<String> {
        self.surface.restore_path().map(|path| format!("/{path}"))
    }
}

/// What one installed application declared, as [`build`] takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    pub service: String,
    pub route: String,
    pub surface: BackupSurface,
}

/// Build the table from what each application declared.
///
/// `wanted` is one [`Wanted`] for every installed application that declared a
/// backup surface **and has a route**. Applications that declared none are not
/// passed in — absence means *offers nothing*, and a caller that filtered
/// correctly cannot express it here.
///
/// Sorted by service name, so the output is a function of the input set rather
/// than of the order a directory happened to be read in.
///
/// # There is no collision case, and that is not an oversight
///
/// [`crate::proxy::build`] refuses two applications asking for one path,
/// because a URL can only reach one of them. Here the key is the **service
/// name**, which the supervisor already keeps unique — two rows can name one
/// route only if the proxy table already refused both, in which case neither is
/// reachable and the rows are inert rather than ambiguous.
#[must_use]
pub fn build(wanted: &[Wanted]) -> Vec<Declaration> {
    let mut sorted: Vec<&Wanted> = wanted.iter().collect();
    sorted.sort_by(|a, b| a.service.cmp(&b.service));
    sorted
        .into_iter()
        .map(|one| Declaration {
            service: one.service.clone(),
            route: one.route.clone(),
            surface: one.surface.clone(),
        })
        .collect()
}

/// The table, as a client reads it.
///
/// One declaration per line:
///
/// ```text
/// tempmon-rivet tempmon artifact sqlite api/v1/backup
/// tempmon-rivet tempmon restore api/v1/restore
/// reliquary-rivet reliquary feed api/v1/blobs
/// ```
///
/// **The shape is a WORD in the third column** and not a flag, for
/// [`crate::proxy::render`]'s reason: the file is read by people as well as by
/// a program, and the word decides how many fields follow it — five for an
/// artifact, four for a feed or a restore. A reader that does not know the word
/// drops the line rather than guessing at its arity.
///
/// A `restore` line follows its application's `artifact` line and is written
/// only when the manifest declared `restore-path` — see [`RESTORE`] for why it
/// is a line of its own.
///
/// It is the same narrow, hand-readable format `crate::proxy` and
/// `crate::settings` use, and for the same reason: `rivet-core` has an empty
/// `[dependencies]` table by rule (`ADR-014` §3), so there is no TOML or JSON
/// parser here and the grammar is as narrow as the emitter.
#[must_use]
pub fn render(declarations: &[Declaration]) -> String {
    let mut out = String::new();
    for one in declarations {
        out.push_str(&one.service);
        out.push(' ');
        out.push_str(&one.route);
        match &one.surface {
            BackupSurface::Artifact {
                path,
                format,
                restore,
            } => {
                out.push(' ');
                out.push_str(ARTIFACT);
                out.push(' ');
                out.push_str(format.as_str());
                out.push(' ');
                out.push_str(path);
                if let Some(restore) = restore {
                    out.push('\n');
                    out.push_str(&one.service);
                    out.push(' ');
                    out.push_str(&one.route);
                    out.push(' ');
                    out.push_str(RESTORE);
                    out.push(' ');
                    out.push_str(restore);
                }
            }
            BackupSurface::Feed { path } => {
                out.push(' ');
                out.push_str(FEED);
                out.push(' ');
                out.push_str(path);
            }
        }
        out.push('\n');
    }
    out
}

/// Read the table back.
///
/// **A malformed line is skipped, not fatal** — `ADR-036` §8's rule, and
/// the same one [`crate::proxy::parse`] follows: a client that refused to read
/// the file over one bad row would fetch nothing from any application on the
/// machine.
///
/// Every field is checked against the same grammar the manifest uses, so a
/// table written by something other than [`render`] cannot introduce a route
/// that escapes its own prefix.
///
/// # A `restore` line attaches to its application's artifact, or to nothing
///
/// `ADR-051` §12. It is collected and attached after every line is read, so
/// the order of lines does not decide it. It attaches only to an `artifact`
/// declaration with the **same service and the same route**; a restore line for
/// a feed, for an application with no artifact line, or a second restore line
/// for one application is dropped — two restore routes for one application is
/// a table this build did not write, and choosing one would make where a
/// restore is sent depend on which line a reader met first. The artifact line
/// itself is kept either way: a malformed restore must not cost the backup.
#[must_use]
pub fn parse(text: &str) -> Vec<Declaration> {
    let mut out = Vec::new();
    let mut restores: Vec<(String, String, String)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split(' ').filter(|f| !f.is_empty());
        let (Some(service), Some(route), Some(shape)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        // **The service field is a directory component on the reading end**
        // (`cmd/cargo-rivet`), so it is checked against the same grammar every
        // real service name already satisfies rather than kept verbatim. A row
        // this build did not write is not assumed to have respected that.
        if crate::service::ServiceName::new(service).is_none() {
            continue;
        }
        if !crate::package::application::is_mountable_proxy_path(route) {
            continue;
        }
        let surface = match shape {
            ARTIFACT => {
                let (Some(format), Some(path)) = (fields.next(), fields.next()) else {
                    continue;
                };
                let Some(format) = BackupFormat::parse(format) else {
                    continue;
                };
                if !crate::package::application::is_proxy_prefix(path) {
                    continue;
                }
                BackupSurface::Artifact {
                    path: path.to_string(),
                    format,
                    restore: None,
                }
            }
            RESTORE => {
                let (Some(path), None) = (fields.next(), fields.next()) else {
                    continue;
                };
                if !crate::package::application::is_proxy_prefix(path) {
                    continue;
                }
                restores.push((service.to_string(), route.to_string(), path.to_string()));
                continue;
            }
            FEED => {
                let Some(path) = fields.next() else {
                    continue;
                };
                if !crate::package::application::is_proxy_prefix(path) {
                    continue;
                }
                BackupSurface::Feed {
                    path: path.to_string(),
                }
            }
            // A shape word this build does not know takes the line with it. A
            // reader cannot know how many fields follow a word it has never
            // seen, so there is nothing to fall back to.
            _ => continue,
        };
        // A trailing field is a line from a producer this build does not
        // understand, and the safe reading of a declaration with more in it
        // than expected is that it was not this declaration.
        if fields.next().is_some() {
            continue;
        }
        out.push(Declaration {
            service: service.to_string(),
            route: route.to_string(),
            surface,
        });
    }
    for (service, route, path) in &restores {
        let named = restores.iter().filter(|(s, _, _)| s == service).count();
        if named != 1 {
            continue;
        }
        // Exactly one artifact line per service is what `render` writes; a
        // table carrying two is not attached to either.
        let mut matching = out.iter_mut().filter(|d| {
            d.service == *service
                && d.route == *route
                && matches!(d.surface, BackupSurface::Artifact { .. })
        });
        if let (Some(one), None) = (matching.next(), matching.next())
            && let BackupSurface::Artifact { restore, .. } = &mut one.surface
        {
            *restore = Some(path.clone());
        }
    }
    out
}

// --- the validation ladder ---------------------------------------------------

/// One rung of [`validate`] that actually ran, in the order they run.
///
/// **Reported rather than assumed.** Not every format has every rung — CSV has
/// neither magic nor a length claim — and a client that printed *"validated"*
/// for a CSV and for a SQLite database would be saying one word about two very
/// different amounts of evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rung {
    /// It is not empty.
    NonEmpty,
    /// As many bytes arrived as the server said it would send.
    Length,
    /// It begins the way this format begins.
    Magic,
    /// The format's own statement about its own size agrees with its size.
    Internal,
}

impl Rung {
    /// What this rung proves, in the words a report prints.
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Self::NonEmpty => "non-empty",
            Self::Length => "bytes received == Content-Length",
            Self::Magic => "format magic",
            Self::Internal => "the format's own internal length claim",
        }
    }
}

/// Why a fetched backup was refused.
///
/// **Every variant carries the numbers**, because the useful thing to say about
/// a backup that did not check out is *by how much* — `AGENTS.md` §5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// Rung 1: nothing arrived. A zero-byte file is a valid empty CSV and is
    /// never a valid backup of anything.
    Empty,
    /// Rung 2: the server said nothing about how long the body was.
    ///
    /// **This is a refusal and not a shrug.** Bytes-received against
    /// `Content-Length` is the rung every format shares, because it asks
    /// nothing about the format — and for CSV nothing after it applies, so a
    /// response with no length is a response whose truncation is undetectable.
    NoLength,
    /// Rung 2: fewer or more bytes arrived than the server declared.
    Truncated { received: u64, declared: u64 },
    /// Rung 3: it does not begin the way this format begins.
    NotTheFormat { format: BackupFormat },
    /// Rung 4: the format's own header says it is a size it is not.
    ///
    /// **This is the rung the ladder exists for.** A truncated SQLite file
    /// still starts with `SQLite format 3\0`, passes a header check, looks like
    /// a backup, and rotates a real one out.
    Incomplete { claimed: u64, received: u64 },
}

impl core::fmt::Display for Fault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => write!(f, "nothing arrived: the body is zero bytes"),
            Self::NoLength => write!(
                f,
                "the response carried no Content-Length, so there is no way to tell a complete \
                 body from a connection that was cut. That is the only check every format has"
            ),
            Self::Truncated { received, declared } => write!(
                f,
                "{received} bytes arrived and the response declared {declared}"
            ),
            Self::NotTheFormat { format } => write!(
                f,
                "it does not begin the way a {format} file begins, so it is not one"
            ),
            Self::Incomplete { claimed, received } => write!(
                f,
                "its own header says it is {claimed} bytes and {received} arrived, so it is a \
                 partial file that carries the right magic"
            ),
        }
    }
}

/// What a fetched artifact was checked for, and what each check proved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The rungs that ran, in order.
    pub rungs: Vec<Rung>,
    /// How many bytes were accepted.
    pub bytes: u64,
}

impl Report {
    /// Whether the format carried a statement about its own completeness.
    ///
    /// **A client should say so when it did not.** For CSV the answer is
    /// always `false`, and that is the honest thing to print beside a stored
    /// file rather than the word *valid*.
    #[must_use]
    pub fn proved_completeness(&self) -> bool {
        self.rungs.contains(&Rung::Internal)
    }
}

/// **The validation ladder** — `ADR-051`, in order, and every rung is
/// format-level.
///
/// 1. **non-empty**
/// 2. **bytes received == `Content-Length`** — generic, free, and the only rung
///    that catches truncation for a format with nothing else
/// 3. **format magic**
/// 4. **the format's own internal length claim**
///
/// # Rung 3 alone is a trap, and the ladder exists because of it
///
/// A truncated SQLite file still starts with `SQLite format 3\0`. It passes a
/// header check, it looks like a backup, and it rotates a real one out. Rung 4
/// is what turns *"this is a SQLite file"* into *"this is a WHOLE SQLite
/// file"*, and for SQLite it is six lines and no library: the header carries
/// the page size at bytes 16–17 and the page count at 28–31, both big-endian,
/// and `page_size × page_count` must equal the file's length.
///
/// # What each format actually gets, stated rather than implied
///
/// | format | rung 3 | rung 4 |
/// | :-- | :-- | :-- |
/// | `sqlite` | `SQLite format 3\0` | page size × page count == length |
/// | `tar` | `ustar` at offset 257 | a multiple of 512 ending in two zero blocks |
/// | `json` | first non-space byte is `{` or `[` | the whole body is one complete, well-formed JSON document |
/// | `csv` | **none — CSV declares nothing about itself** | **none** |
///
/// CSV gets rungs 1 and 2 and nothing else, and [`Report::rungs`] says so. A
/// client that printed one word for all four formats would be claiming evidence
/// it does not have.
///
/// # Errors
///
/// [`Fault`], naming the rung that refused and the numbers that disagreed.
pub fn validate(
    format: BackupFormat,
    bytes: &[u8],
    content_length: Option<u64>,
) -> Result<Report, Fault> {
    let received = bytes.len() as u64;
    let mut rungs = Vec::new();

    // Rung 1.
    if bytes.is_empty() {
        return Err(Fault::Empty);
    }
    rungs.push(Rung::NonEmpty);

    // Rung 2. Absent is a refusal — see `Fault::NoLength`.
    let declared = content_length.ok_or(Fault::NoLength)?;
    if declared != received {
        return Err(Fault::Truncated { received, declared });
    }
    rungs.push(Rung::Length);

    match format {
        BackupFormat::Sqlite => {
            // Rung 3. The 16-byte header string, NUL included.
            const MAGIC: &[u8] = b"SQLite format 3\0";
            if bytes.len() < 32 || !bytes.starts_with(MAGIC) {
                return Err(Fault::NotTheFormat { format });
            }
            rungs.push(Rung::Magic);

            // Rung 4. The header's own arithmetic, big-endian throughout.
            //
            // **Bytes 16-17 are the page size and 1 means 65536**, which is the
            // format's own escape for a value that does not fit in two bytes.
            // A reader that took the 1 literally would compute a length 65,535
            // times too small and refuse every large database.
            let raw = u32::from(u16::from_be_bytes([bytes[16], bytes[17]]));
            let page_size = if raw == 1 { 65_536 } else { raw };
            let page_count = u32::from_be_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]);
            let claimed = u64::from(page_size) * u64::from(page_count);
            // **A page count of zero is not a claim.** SQLite wrote it before
            // version 3.7.0 and a file carrying it says nothing about its own
            // size, so rung 4 does not run rather than passing vacuously.
            if page_count != 0 {
                if claimed != received {
                    return Err(Fault::Incomplete { claimed, received });
                }
                rungs.push(Rung::Internal);
            }
        }
        BackupFormat::Tar => {
            // Rung 3. `ustar` at offset 257 of the first header block — POSIX
            // and GNU both write it, and a tar with no member at all still has
            // the end-of-archive blocks rather than this, which is why a short
            // archive is refused here rather than at rung 4.
            if bytes.len() < 262 || &bytes[257..262] != b"ustar" {
                return Err(Fault::NotTheFormat { format });
            }
            rungs.push(Rung::Magic);

            // Rung 4. **tar's own end-of-archive marker**: the file is a whole
            // number of 512-byte blocks and the last two are zero. That is the
            // format saying *this is the end*, which is exactly the claim a
            // truncated archive cannot make.
            let whole_blocks = received.is_multiple_of(512);
            let tail_is_zero = bytes.len() >= 1024 && bytes[bytes.len() - 1024..].iter().all(|b| *b == 0);
            if !whole_blocks || !tail_is_zero {
                // The claim is *what a complete archive would weigh at least*,
                // which is what a reader wants to compare against.
                return Err(Fault::Incomplete {
                    claimed: received.next_multiple_of(512).max(1024),
                    received,
                });
            }
            rungs.push(Rung::Internal);
        }
        BackupFormat::Json => {
            // Rung 3. JSON has no magic number; what it has is a grammar whose
            // first token is decidable. A document is an object or an array —
            // a bare scalar is legal JSON and is not a backup of anything.
            let first = bytes.iter().find(|b| !b.is_ascii_whitespace());
            if !matches!(first, Some(b'{') | Some(b'[')) {
                return Err(Fault::NotTheFormat { format });
            }
            rungs.push(Rung::Magic);

            // Rung 4. The whole body, walked as JSON syntax — see
            // `json_is_complete`'s own documentation for why the first and
            // last byte alone are not enough.
            match json_is_complete(bytes) {
                Ok(()) => rungs.push(Rung::Internal),
                Err(JsonSyntaxFault::RanOut) => {
                    return Err(Fault::Incomplete {
                        claimed: 0,
                        received,
                    });
                }
                Err(JsonSyntaxFault::Malformed) => {
                    return Err(Fault::NotTheFormat { format });
                }
            }
        }
        // **Neither rung exists for CSV, and inventing one would be worse than
        // having none.** A trailing newline is a convention rather than the
        // format, and a check on the column count of the last row is a check on
        // a SCHEMA — which is the line this whole design refuses to cross.
        BackupFormat::Csv => {}
    }

    Ok(Report {
        rungs,
        bytes: received,
    })
}

// --- rung 4 for JSON: a full syntax check, iterative rather than a tree -----

/// Why [`json_is_complete`] refused a body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsonSyntaxFault {
    /// The scanner needed another byte — to close a string, a container, a
    /// literal or a number — and the body had none left. **Every container
    /// still open when this happens is the same shape as a stream cut mid
    /// record**, which is why this is [`Fault::Incomplete`] rather than a
    /// syntax complaint.
    RanOut,
    /// A byte was there and it was the wrong one: invalid UTF-8, a bad
    /// escape, a missing `:` or `,`, or data after the top-level value
    /// closed. [`Fault::NotTheFormat`], the same as rung 3's.
    Malformed,
}

/// One open container, as [`json_is_complete`]'s explicit stack holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsonFrame {
    Object,
    Array,
}

/// What the scanner will accept at the current position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsonExpect {
    /// A value must start, and the innermost container's own close is also
    /// legal here — true only right after `{` or `[`.
    ValueOrClose,
    /// A value must start; the innermost container's close is not legal here
    /// — right after `,` in an array, and right after `:` anywhere.
    Value,
    /// An object key (`"..."`) must start, or the object may close
    /// immediately — true only right after `{`.
    KeyOrClose,
    /// An object key must start — right after `,` in an object.
    Key,
    /// `:` must be next.
    Colon,
    /// A value just finished: `,` or the innermost container's own close.
    CommaOrClose,
}

/// **Full JSON syntax, checked with an explicit stack instead of the call
/// stack.** Rung 4 for [`BackupFormat::Json`] — see [`validate`]'s own
/// documentation for the ladder this belongs to.
///
/// Unlike [`crate::json::parse`], this builds no tree and has no depth cap:
/// nesting lives in a `Vec`, so a hundred thousand levels of `[[[[…` are a
/// hundred thousand heap entries rather than a hundred thousand stack frames.
/// It also answers a narrower question than a reader needs to — *is this one
/// complete, well-formed document?* — and keeps nothing it reads, which is
/// what makes it suitable for a backup body a reader would refuse to hold a
/// tree of at all.
///
/// **The two ways this refuses are not the same finding.** `{"records":[{"a":1}`
/// is truncated mid-array mid-object: every byte the scanner met was the
/// right one, and it simply ran out — [`JsonSyntaxFault::RanOut`], the same
/// shape as a stream cut mid-record. `{"a":}` is not truncated at all — a
/// byte is there and it is the wrong one — so it is
/// [`JsonSyntaxFault::Malformed`] instead, whatever a matching first and last
/// byte might have suggested.
///
/// # Errors
///
/// [`JsonSyntaxFault`].
fn json_is_complete(bytes: &[u8]) -> Result<(), JsonSyntaxFault> {
    // JSON is Unicode text; a body that is not valid UTF-8 is not JSON,
    // whatever its bytes look like structurally.
    let text = core::str::from_utf8(bytes).map_err(|_| JsonSyntaxFault::Malformed)?;
    let b = text.as_bytes();
    let len = b.len();

    let mut i = skip_json_ws(b, 0);
    let mut stack: Vec<JsonFrame> = Vec::new();
    // The top level must be one object or one array — rung 3's rule, kept
    // here too so this function is correct standing alone.
    let mut expect = match b.get(i) {
        Some(b'{') => {
            stack.push(JsonFrame::Object);
            i += 1;
            JsonExpect::KeyOrClose
        }
        Some(b'[') => {
            stack.push(JsonFrame::Array);
            i += 1;
            JsonExpect::ValueOrClose
        }
        Some(_) => return Err(JsonSyntaxFault::Malformed),
        None => return Err(JsonSyntaxFault::RanOut),
    };

    loop {
        i = skip_json_ws(b, i);
        let Some(byte) = b.get(i).copied() else {
            // The stack is never empty here: emptying it breaks out of the
            // loop below instead of looping back for another byte.
            return Err(JsonSyntaxFault::RanOut);
        };
        match expect {
            JsonExpect::KeyOrClose => {
                if byte == b'}' {
                    i += 1;
                    stack.pop();
                    match stack.is_empty() {
                        true => break,
                        false => expect = JsonExpect::CommaOrClose,
                    }
                } else if byte == b'"' {
                    i = json_skip_string(text, i)?;
                    expect = JsonExpect::Colon;
                } else {
                    return Err(JsonSyntaxFault::Malformed);
                }
            }
            JsonExpect::Key => {
                if byte == b'"' {
                    i = json_skip_string(text, i)?;
                    expect = JsonExpect::Colon;
                } else {
                    return Err(JsonSyntaxFault::Malformed);
                }
            }
            JsonExpect::Colon => {
                if byte != b':' {
                    return Err(JsonSyntaxFault::Malformed);
                }
                i = skip_json_ws(b, i + 1);
                i = json_start_value(text, i, &mut stack, &mut expect)?;
            }
            JsonExpect::ValueOrClose => {
                if byte == b']' {
                    i += 1;
                    stack.pop();
                    match stack.is_empty() {
                        true => break,
                        false => expect = JsonExpect::CommaOrClose,
                    }
                } else {
                    i = json_start_value(text, i, &mut stack, &mut expect)?;
                }
            }
            JsonExpect::Value => {
                i = json_start_value(text, i, &mut stack, &mut expect)?;
            }
            // `.copied()` so the match holds no borrow of `stack` — the
            // close arms below need to `pop` it, and `JsonFrame` is `Copy`
            // precisely so this can be an owned value instead.
            JsonExpect::CommaOrClose => match (byte, stack.last().copied()) {
                (b',', Some(JsonFrame::Object)) => {
                    i += 1;
                    expect = JsonExpect::Key;
                }
                (b',', Some(JsonFrame::Array)) => {
                    i += 1;
                    expect = JsonExpect::Value;
                }
                (b'}', Some(JsonFrame::Object)) | (b']', Some(JsonFrame::Array)) => {
                    i += 1;
                    stack.pop();
                    match stack.is_empty() {
                        true => break,
                        false => expect = JsonExpect::CommaOrClose,
                    }
                }
                _ => return Err(JsonSyntaxFault::Malformed),
            },
        }
    }

    // The top-level container just closed. Only whitespace may follow it —
    // a second document, or any other trailing byte, is data this body never
    // claimed to be part of it.
    let i = skip_json_ws(b, i);
    if i == len {
        Ok(())
    } else {
        Err(JsonSyntaxFault::Malformed)
    }
}

fn skip_json_ws(b: &[u8], mut i: usize) -> usize {
    // The four RFC 8259 permits, and no others — the same four
    // `crate::json`'s own `skip_whitespace` allows.
    while matches!(b.get(i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
        i += 1;
    }
    i
}

/// Start a value at `i` — an object, an array, a string, a number, or one of
/// the three literals — and advance `expect` to what should follow it.
///
/// A container start pushes `stack` and leaves `expect` set for the newly
/// opened container's own first position; every other value sets `expect` to
/// [`JsonExpect::CommaOrClose`], because a scalar is always immediately
/// followed by a delimiter or the innermost container's close.
fn json_start_value(
    text: &str,
    i: usize,
    stack: &mut Vec<JsonFrame>,
    expect: &mut JsonExpect,
) -> Result<usize, JsonSyntaxFault> {
    let b = text.as_bytes();
    let Some(byte) = b.get(i).copied() else {
        return Err(JsonSyntaxFault::RanOut);
    };
    match byte {
        b'{' => {
            stack.push(JsonFrame::Object);
            *expect = JsonExpect::KeyOrClose;
            Ok(i + 1)
        }
        b'[' => {
            stack.push(JsonFrame::Array);
            *expect = JsonExpect::ValueOrClose;
            Ok(i + 1)
        }
        b'"' => {
            let end = json_skip_string(text, i)?;
            *expect = JsonExpect::CommaOrClose;
            Ok(end)
        }
        b't' => {
            let end = json_skip_literal(text, i, "true")?;
            *expect = JsonExpect::CommaOrClose;
            Ok(end)
        }
        b'f' => {
            let end = json_skip_literal(text, i, "false")?;
            *expect = JsonExpect::CommaOrClose;
            Ok(end)
        }
        b'n' => {
            let end = json_skip_literal(text, i, "null")?;
            *expect = JsonExpect::CommaOrClose;
            Ok(end)
        }
        b'-' | b'0'..=b'9' => {
            let end = json_skip_number(text, i)?;
            *expect = JsonExpect::CommaOrClose;
            Ok(end)
        }
        _ => Err(JsonSyntaxFault::Malformed),
    }
}

/// Consume a JSON string starting at its opening `"`, and return the index
/// just past the closing one. **Content is discarded** — this is a syntax
/// check, not a reader — but every rule a reader would enforce still runs:
/// every escape JSON defines, `\uXXXX` as four hex digits, and no raw control
/// character.
fn json_skip_string(text: &str, mut i: usize) -> Result<usize, JsonSyntaxFault> {
    let b = text.as_bytes();
    i += 1; // the opening quote; `json_start_value` and the object-key
    // callers only reach here on a byte already known to be `"`.
    loop {
        let Some(byte) = b.get(i).copied() else {
            return Err(JsonSyntaxFault::RanOut);
        };
        match byte {
            b'"' => return Ok(i + 1),
            // RFC 8259 forbids these unescaped.
            0x00..=0x1F => return Err(JsonSyntaxFault::Malformed),
            b'\\' => {
                let Some(esc) = b.get(i + 1).copied() else {
                    return Err(JsonSyntaxFault::RanOut);
                };
                match esc {
                    b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => i += 2,
                    b'u' => {
                        for k in 0..4 {
                            let Some(hex) = b.get(i + 2 + k).copied() else {
                                return Err(JsonSyntaxFault::RanOut);
                            };
                            if !hex.is_ascii_hexdigit() {
                                return Err(JsonSyntaxFault::Malformed);
                            }
                        }
                        i += 6;
                    }
                    _ => return Err(JsonSyntaxFault::Malformed),
                }
            }
            _ => {
                // An ordinary byte of a UTF-8 encoded character. `text` was
                // already proven valid UTF-8 whole, and `i` only ever moves by
                // whole characters, so it is always on a boundary here.
                let step = text[i..].chars().next().map_or(1, char::len_utf8);
                i += step;
            }
        }
    }
}

/// Match `word` (`"true"`, `"false"` or `"null"`) starting at `i`, and return
/// the index just past it.
///
/// **Distinguishes a truncated literal from a wrong one.** `"tru"` at the end
/// of the body is [`JsonSyntaxFault::RanOut`] — every byte present agreed with
/// `word` and there were simply no more — while `"trx"` is
/// [`JsonSyntaxFault::Malformed`], because a byte is there and it disagrees.
fn json_skip_literal(text: &str, i: usize, word: &str) -> Result<usize, JsonSyntaxFault> {
    let b = text.as_bytes();
    let end = i + word.len();
    if end > b.len() {
        return if b[i..].starts_with(&word.as_bytes()[..b.len() - i]) {
            Err(JsonSyntaxFault::RanOut)
        } else {
            Err(JsonSyntaxFault::Malformed)
        };
    }
    if &b[i..end] == word.as_bytes() {
        Ok(end)
    } else {
        Err(JsonSyntaxFault::Malformed)
    }
}

/// Consume a JSON number starting at its first byte (`-` or a digit), and
/// return the index just past it.
///
/// The same grammar [`crate::json`]'s own reader uses: a leading zero is a
/// complete integer part on its own and is never followed by another digit —
/// so `01` is read as the number `0` followed by a stray `1`, which is
/// refused wherever that `1` turns out to be — a fraction needs at least one
/// digit after its `.`, and an exponent needs at least one digit after its
/// optional sign.
fn json_skip_number(text: &str, mut i: usize) -> Result<usize, JsonSyntaxFault> {
    let b = text.as_bytes();
    if b.get(i) == Some(&b'-') {
        i += 1;
    }
    i = json_digits(b, i, true)?;
    if b.get(i) == Some(&b'.') {
        i = json_digits(b, i + 1, false)?;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        i = json_digits(b, i, false)?;
    }
    Ok(i)
}

/// One run of digits at `i`, at least one of them. `leading` allows the
/// `0`-is-a-complete-integer-on-its-own rule; every other caller wants a
/// fraction or an exponent, where a leading zero is an ordinary digit.
fn json_digits(b: &[u8], i: usize, leading: bool) -> Result<usize, JsonSyntaxFault> {
    match b.get(i) {
        Some(b'0') if leading => Ok(i + 1),
        Some(b'0'..=b'9') => {
            let mut i = i + 1;
            while matches!(b.get(i), Some(b'0'..=b'9')) {
                i += 1;
            }
            Ok(i)
        }
        Some(_) => Err(JsonSyntaxFault::Malformed),
        None => Err(JsonSyntaxFault::RanOut),
    }
}

#[cfg(test)]
mod tests;
