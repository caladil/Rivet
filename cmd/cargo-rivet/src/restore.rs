//! `cargo rivet restore` — **send one application its own backup back.**
//! `ADR-051` §12, `.rulings/` R-183 D1.
//!
//! `cargo rivet backup` fetches what an application's `backup-path` serves.
//! This is the other direction: the application declares `restore-path` in its
//! signed manifest, and this verb sends a file to it **over the control plane**
//! (`.rulings/` R-234) — `POST /restore/<service>` on port 7777, which the
//! machine relays to the application over loopback, exactly as `rivet-proxyd`
//! would, and whose answer it streams back. The same machine table and the same
//! transport as `backup`, built by the same functions in `crate::backup`.
//!
//! # The application restores itself, and Rivet stops nothing
//!
//! R-183 D1: *"the application handles its own consistency."* A restore that
//! Rivet performed — stop the service, overwrite a file in its data directory,
//! start it — would need Rivet to know which file, and when replacing it is
//! safe, which is exactly the knowledge `ADR-051` §9 rejected Rivet taking a
//! backup without. So this verb sends bytes to a route and reports what the
//! application said. **It never calls `apply service-stop`**, and the
//! application's contract (`ADR-051` §12) is to validate the whole body before
//! it changes anything, replace its data as one step, and answer only when that
//! step is done.
//!
//! # What is checked before a byte is sent
//!
//! In order, and each refusal sends nothing:
//!
//! 1. the file exists, is a regular file, and is no larger than
//!    `crate::backup::MAX_BODY`;
//! 2. the machine's table names the application (the refusal names what it
//!    does offer), and it declared the artifact shape and `restore-path`;
//! 3. the file passes `rivet_core::backup::validate` against the application's
//!    DECLARED `backup-format` — the same ladder `backup` runs on what it
//!    fetches, so a client cannot hand an application a truncated file and call
//!    it a restore;
//! 4. the file does not sit in a directory named for a DIFFERENT application
//!    this machine offers — see `other_application` for exactly how weak
//!    that is.
//!
//! # What "matches the format" can honestly mean
//!
//! `validate`'s rungs, and nothing more. Rung 2 is weaker here than on a fetch:
//! there is no server's `Content-Length`, so it compares the bytes read with
//! the size the filesystem reported when the file was opened — it catches a
//! file that changed while it was read, and nothing else. **No format carries
//! which application wrote it** — a SQLite header, a tar archive, a JSON
//! document and a CSV file say nothing about their producer — so whether this
//! is TempMon's database or another SQLite application's is not something the
//! bytes can answer. The report says which rungs ran.
//!
//! # The file is read whole here, and that is what makes the check mean something
//!
//! The file is read whole, checked, and sent from the same buffer with a
//! `Content-Length`. What reaches the application is byte for byte what the
//! ladder passed — a send that checked one read and sent a second would be
//! checking a file rather than a request. The machine streams it on to the
//! application without holding it.

use std::path::{Path, PathBuf};

use rivet_core::backup::Declaration;
use rivet_core::package::application::{BackupFormat, BackupSurface};
use rivet_remote::endpoint::{Endpoint, IDENTITY_AND_PIN};
use rivet_remote::tls::CallsTools;

use crate::backup::{MAX_BODY, ReachesApplications, Relayed};

/// How much of an application's answer is printed.
///
/// An answer is a sentence or a small JSON report — TempMon's names the rows it
/// discarded and loaded. A bound exists because the bytes come from another
/// machine; past it the report says how much was left out.
const MAX_ANSWER: u64 = 64 * 1024;

pub fn usage() -> String {
    format!(
        "usage: cargo rivet restore --host HOST:PORT --identity DIR --pin HEX --service NAME --from FILE [confirm]\n\
         \n\
         Sends ONE application a backup of its own data, over the control plane:\n\
         the machine relays it to the application over loopback. The application\n\
         declares where it accepts one with `restore-path` in its signed\n\
         manifest, and it replaces its own data -- ADR-051 §12.\n\
         \n\
         WITHOUT `confirm` NOTHING IS SENT. It reads the machine's table, checks\n\
         the file, and prints where it would be sent.\n\
         \n\
         restore options:\n\
         \x20 --host HOST:PORT   REQUIRED. Where rivet-mcpd is listening. The\n\
         \x20                    table and the restore both go over it.\n\
         {IDENTITY_AND_PIN}\
         \x20 --service NAME     REQUIRED, once. The application to restore.\n\
         \x20 --from FILE        REQUIRED. The backup file -- what `cargo rivet\n\
         \x20                    backup` stored for that application.\n\
         \x20 confirm            send it. Without it, the plan.\n\
         \n\
         Nothing but your control-plane identity is presented, so --authority,\n\
         --name and --port -- which named the fleet's browser certificate and\n\
         the proxy on 443 -- are REFUSED, not ignored (R-234).\n\
         \n\
         REFUSED BEFORE ANYTHING IS SENT:\n\
         \x20 - an application the machine does not offer, or one that declares no\n\
         \x20   restore-path (or declares a feed, which has none);\n\
         \x20 - a file that fails the application's declared backup-format, by the\n\
         \x20   same ladder `backup` runs: non-empty, the bytes read match the file's\n\
         \x20   size, the format's magic, the format's own statement about its size.\n\
         \x20   CSV has only the first two, and the plan says so;\n\
         \x20 - a file larger than {MAX_BODY} bytes -- it is read whole, checked, and\n\
         \x20   sent from that one buffer, so what is sent is what was checked;\n\
         \x20 - a file inside a directory named for a DIFFERENT application this\n\
         \x20   machine offers, which is where `backup` would have put that one's.\n\
         \x20   No format records which application wrote it, so nothing stronger\n\
         \x20   is possible.\n\
         \n\
         NOTHING ON THE MACHINE IS STOPPED. The application restores itself:\n\
         it checks the body, replaces its data, and answers. Its HTTP answer is\n\
         printed as it gave it, and anything but 2xx is a failure.\n\
         NOTHING HERE VALIDATES A SCHEMA. Whether an old backup fits a newer\n\
         application is the application's own refusal to make.\n"
    )
}

/// What `restore` was asked to do.
#[derive(Debug)]
pub struct RestoreArgs {
    pub endpoint: Endpoint,
    pub service: String,
    pub from: PathBuf,
    pub confirm: bool,
}

/// # Errors
///
/// A flag with no value, a flag this verb does not take, `--service` given
/// twice, a missing `--service` or `--from`, or either half of `ADR-025` §2's
/// trust without the other.
pub fn parse(args: &[String]) -> Result<RestoreArgs, String> {
    let mut endpoint = Endpoint::default();
    let mut service: Option<String> = None;
    let mut from: Option<PathBuf> = None;
    let mut confirm = false;

    let mut index = 0;
    while index < args.len() {
        if let Some(next) = endpoint.read(args, index)? {
            index = next;
            continue;
        }
        let value = |flag: &str| -> Result<String, String> {
            args.get(index + 1)
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match args[index].as_str() {
            // **Once.** `backup --service` is repeatable because a backup of
            // several applications is several independent fetches; a restore
            // replaces one application's data, and a second name on the line
            // is far more likely a mistake than a wish.
            "--service" => {
                let named = value("--service")?;
                if let Some(first) = &service {
                    return Err(format!(
                        "--service was given twice ({first} and {named}). A restore replaces ONE \
                         application's data; run it once per application."
                    ));
                }
                service = Some(named);
                index += 2;
            }
            "--from" => {
                from = Some(PathBuf::from(value("--from")?));
                index += 2;
            }
            flag @ ("--authority" | "--name" | "--port") => {
                return Err(crate::backup::retired_flag(flag, "restore"));
            }
            "confirm" => {
                confirm = true;
                index += 1;
            }
            other => {
                return Err(format!(
                    "{other:?} is not an option `restore` takes.\n\n{}",
                    usage()
                ));
            }
        }
    }
    endpoint.paired()?;

    let service = service.ok_or_else(|| {
        format!(
            "--service is required: a restore replaces one application's data, and choosing \
             which is not this tool's to guess.\n\n{}",
            usage()
        )
    })?;
    let from = from.ok_or_else(|| {
        format!(
            "--from is required: it is the backup file to send.\n\n{}",
            usage()
        )
    })?;

    Ok(RestoreArgs {
        endpoint,
        service,
        from,
        confirm,
    })
}

// ---------------------------------------------------------------------------
// Checks
// ---------------------------------------------------------------------------

/// The declaration for `service`, if the machine offers a restore for it.
///
/// # Errors
///
/// An application the table does not name (naming what it does), a feed, or
/// an artifact with no `restore-path`.
fn find_restorable<'a>(
    table: &'a [Declaration],
    service: &str,
) -> Result<(&'a Declaration, BackupFormat, String), String> {
    let Some(one) = table.iter().find(|d| d.service == service) else {
        let offered: Vec<&str> = table.iter().map(|d| d.service.as_str()).collect();
        return Err(format!(
            "{service} is not an application this machine offers a backup for, so it offers no \
             restore either. NOTHING WAS SENT.\n\
             \x20 asked    {service}\n\
             \x20 offers   {}\n\
             \x20 why      an application offers a restore by declaring restore-path beside \
             backup-path and backup-format in its signed manifest (ADR-051 §12); a name renamed on \
             one side and not the other matches nothing.",
            if offered.is_empty() {
                "nothing -- no application on this machine offers a backup at all".to_string()
            } else {
                offered.join(", ")
            }
        ));
    };
    match &one.surface {
        BackupSurface::Feed { .. } => Err(format!(
            "{service} offers a backup FEED, which has no restore: a restore sends back what one \
             fetch of backup-path returned, checked against backup-format, and a feed is neither \
             (ADR-051 §12). NOTHING WAS SENT."
        )),
        BackupSurface::Artifact { format, .. } => {
            let path = one.restore_request_path().ok_or_else(|| {
                format!(
                    "{service} offers a backup ({format}, at {}) and NO RESTORE: its signed \
                     manifest declares no restore-path. NOTHING WAS SENT.\n\
                     \x20 why  Rivet does not write an application's data itself -- the \
                     application restores its own, at a route it declares (ADR-051 §12). Until \
                     it declares one, putting this file back is that application's own tooling.",
                    one.request_path()
                )
            })?;
            Ok((one, *format, path))
        }
    }
}

/// **The one check this verb can make about WHICH application a file is from,
/// and how weak it is.**
///
/// No backup format records its producer, so the bytes cannot say. What
/// `cargo rivet backup` does say is where it put a file:
/// `<into>/<service>/<file>`. So a file whose parent directory is named for a
/// DIFFERENT application this machine offers is, in the ordinary case, that
/// application's backup — and sending it here is refused.
///
/// **It proves nothing in the other direction.** A file copied anywhere else,
/// renamed, or sitting in a directory named for no offered application passes.
/// It catches the slip of typing one application's name and the other's path,
/// and that is all it is claimed to catch.
fn other_application<'a>(from: &Path, service: &str, table: &'a [Declaration]) -> Option<&'a str> {
    let parent = from.parent()?.file_name()?.to_str()?;
    table
        .iter()
        .map(|d| d.service.as_str())
        .find(|other| *other != service && *other == parent)
}

/// What a rung proved, in a restore's words. Rung 2 is not a server's
/// `Content-Length` here — see the module note.
fn describe(rung: rivet_core::backup::Rung) -> &'static str {
    match rung {
        rivet_core::backup::Rung::Length => "bytes read == the file's size",
        other => other.describe(),
    }
}

/// The file, read whole and checked.
///
/// # Errors
///
/// A path that cannot be opened or is not a regular file, a file over
/// [`MAX_BODY`], a read that fails, or a rung of the ladder.
fn read_and_check(
    from: &Path,
    format: BackupFormat,
    service: &str,
) -> Result<(Vec<u8>, rivet_core::backup::Report), String> {
    let meta = std::fs::metadata(from).map_err(|e| {
        format!("{}: {e}. NOTHING WAS SENT.", from.display())
    })?;
    if !meta.is_file() {
        return Err(format!(
            "{} is not a regular file. NOTHING WAS SENT.",
            from.display()
        ));
    }
    if meta.len() > MAX_BODY {
        return Err(format!(
            "{} is {} bytes, over the {MAX_BODY}-byte bound. NOTHING WAS SENT.\n\
             \x20 why  it is read whole, checked, and sent from that one buffer, and `cargo rivet \
             backup` refuses to fetch a body this large for the same reason.",
            from.display(),
            meta.len()
        ));
    }
    let bytes = std::fs::read(from).map_err(|e| format!("{}: {e}", from.display()))?;
    let report = rivet_core::backup::validate(format, &bytes, Some(meta.len())).map_err(|fault| {
        format!(
            "{}: it is NOT a whole {format} file, which is what {service} declares its backup to \
             be: {fault}. NOTHING WAS SENT.\n\
             \x20 why  a file that fails a rung of ADR-051's ladder looks like a backup and is \
             not one, and an application handed it would be asked to replace its data with it.",
            from.display()
        )
    })?;
    Ok((bytes, report))
}

// ---------------------------------------------------------------------------
// The verb
// ---------------------------------------------------------------------------

/// `restore`, with a real connection behind it.
///
/// # Errors
///
/// See [`restore_via`].
pub fn execute(args: &[String]) -> Result<String, String> {
    let parsed = parse(args)?;
    let transport = parsed.endpoint.open()?.ok_or_else(|| {
        format!(
            "--identity and --pin are required: a restore goes over ADR-035's control plane, \
             and there is no second way to ask.\n\n{}",
            usage()
        )
    })?;
    let host = transport.host.clone();
    restore_via(&parsed, &transport, &transport, &host)
}

/// **The whole restore, against any [`CallsTools`] and any
/// [`ReachesApplications`].**
///
/// `reach` is asked only after every check has passed and `confirm` was given,
/// so a plan and a refusal send nothing.
///
/// # Errors
///
/// Every refusal in the module note, a table this build cannot read, a
/// transport failure, the machine's own refusal, or an answer that is not 2xx.
pub(crate) fn restore_via(
    args: &RestoreArgs,
    caller: &dyn CallsTools,
    reach: &dyn ReachesApplications,
    host: &str,
) -> Result<String, String> {
    // The file's existence first: a typo in --from costs no network at all.
    if !args.from.is_file() {
        return Err(format!(
            "{} is not a file this workstation can read. NOTHING WAS SENT.",
            args.from.display()
        ));
    }
    let table = crate::backup::read_table(caller)?;
    let (declaration, format, restore_path) = find_restorable(&table, &args.service)?;
    let (bytes, report) = read_and_check(&args.from, format, &args.service)?;
    if let Some(other) = other_application(&args.from, &args.service, &table) {
        return Err(format!(
            "{} is inside a directory named {other}, which is another application on this \
             machine -- `cargo rivet backup` puts each application's backup in a directory of its \
             own name. NOTHING WAS SENT to {}.\n\
             \x20 why  no backup format records which application wrote it, so where the file \
             sits is the one sign available, and restoring one application's data into another \
             replaces the second's.\n\
             \x20 fix  if this really is {}'s backup, copy it out of that directory first.",
            args.from.display(),
            args.service,
            args.service
        ));
    }

    let target = format!("{restore_path}?{}", rivet_core::backup::RESTORE_QUERY);
    let media = format.media_type();
    let checked = report
        .rungs
        .iter()
        .map(|r| describe(*r))
        .collect::<Vec<_>>()
        .join("; ");
    let mut out = format!(
        "restore {}\n\
         \x20 from    {}  {} bytes\n\
         \x20 checked {checked}\n",
        declaration.service,
        args.from.display(),
        report.bytes
    );
    if !report.proved_completeness() {
        out.push_str(&format!(
            "\x20 NOTE    {format} makes no statement about its own size, so nothing here proves \
             the file is whole beyond its size on disk.\n"
        ));
    }
    out.push_str(&format!(
        "\x20 send    POST {} on {host}, which relays it over loopback to the application's \
         restore route, {} {target}\n\
         \x20 as      Content-Type: {media}\n\
         \x20 then    {} checks the body and replaces its own data. Nothing on the machine is \
         stopped or restarted by this verb.\n",
        rivet_core::backup::restore_route(&declaration.service),
        rivet_core::backup::RESTORE_METHOD,
        declaration.service
    ));

    if !args.confirm {
        out.push_str("\nNothing was sent. Add `confirm` to send it.\n");
        return Ok(out);
    }

    let answer = reach.restore(&declaration.service, &bytes, MAX_ANSWER)?;
    if answer.answered_by_the_machine() {
        return Err(format!(
            "{out}\nTHE RESTORE FAILED: the machine answered {} itself, not {}:\n{}",
            answer.status,
            declaration.service,
            answer.text()
        ));
    }
    out.push_str(&format!("\n{}\n", render_answer(&declaration.service, &answer)));
    if (200..300).contains(&answer.status) {
        if answer.status == 202 {
            out.push_str(
                "NOTE: 202 means the application ACCEPTED the file and says it has not finished. \
                 ADR-051 §12 asks for an answer only once the data is replaced; this one did not \
                 say that.\n",
            );
        }
        Ok(out)
    } else {
        Err(format!(
            "{out}\nTHE RESTORE FAILED: {} answered {}, and anything but 2xx is a failure. The \
             application's contract is to change nothing when it refuses (ADR-051 §12); only \
             it can say whether it kept to that.",
            declaration.service, answer.status
        ))
    }
}

/// The application's answer, as it gave it.
fn render_answer(service: &str, answer: &Relayed) -> String {
    let mut out = format!(
        "{service} answered {}{}\n",
        answer.status,
        answer
            .header("content-type")
            .map(|t| format!(" ({t})"))
            .unwrap_or_default()
    );
    if answer.body.is_empty() {
        out.push_str("  (an empty body)\n");
    } else {
        match std::str::from_utf8(&answer.body) {
            Ok(text) => {
                for line in text.lines() {
                    out.push_str("  | ");
                    out.push_str(line);
                    out.push('\n');
                }
            }
            Err(_) => out.push_str(&format!(
                "  ({} bytes that are not text, not printed)\n",
                answer.body.len()
            )),
        }
    }
    if answer.truncated {
        out.push_str(&format!(
            "  (the answer was longer than {MAX_ANSWER} bytes; the rest is not shown)\n"
        ));
    }
    out
}

#[cfg(test)]
#[path = "restore_tests.rs"]
mod tests;
