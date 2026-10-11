//! **`cargo rivet secrets` — the three TLS files an application needs, from
//! the authority on this workstation into that application's data directory.**
//!
//! `ADR-046` step 3, and it is the same defect [`crate::credentials`] was
//! written for one layer down: *every capability already existed and nothing
//! sequenced them.*
//!
//! # What was actually missing, stated exactly
//!
//! `STATE.md` said *"nothing mints or installs them"*. Half of that was never
//! true and the other half is what this file is:
//!
//! * **Nothing needs minting.** `ADR-046` §4.2 is explicit that a backend
//!   serves *"a certificate for the name the proxy verifies it against — the
//!   machine's one fleet name"*, and the proxy presents *"the same pair it
//!   serves to browsers, because the machine has one name and one
//!   certificate"*. That pair already exists: `rivet-provision ca server` minted
//!   it, and it is in the authority directory beside the root.
//! * **Installing already worked.** `apply install-secret` has existed since
//!   2026-09-01 and `rivet-proxyd` on `rivet-01` is running on exactly these
//!   three files — proved by reading its data directory, where `tls-cert`,
//!   `tls-key` and `tls-root` are 524, 246 and 610 bytes, which is byte for
//!   byte what is on this host.
//!
//! What did not exist is one command. Doing it by hand is three
//! `apply install-secret` calls per application, each carrying a few thousand
//! characters of hex that somebody has to produce — **and one of them is a
//! private key, which then sits in a shell's history and in the process table
//! of whatever ran it.** This reads the files itself and never builds a shell
//! command, so the key is in this process and in the TLS session and nowhere
//! else.
//!
//! # It installs the MACHINE's certificate, and that is the design
//!
//! Not a per-application leaf. Every backend on this board answers to the same
//! name, because there is one machine and the proxy verifies one name against
//! all of them (`upstream-name`). A leaf per application would be several
//! private keys where the design has one, each needing its own renewal, to
//! express a distinction `ADR-046` deliberately does not draw.
//!
//! **The same pair also serves as a client certificate**, which is the other
//! half of `ADR-046` §4.2: an application that calls another — `aura-controller`
//! reaching TempMon — presents it. The leaves this authority mints carry no
//! extended key usage, so they are valid for both, and `cmd/rivet-provision`'s
//! `ca` module is where that is decided.
//!
//! # What it does not do
//!
//! **It does not restart anything.** `apply install-secret` says so in its own
//! answer, and it is right to: an application reads its TLS files at start, so
//! the operator decides when a service that is serving stops serving.
//! `apply service-restart <name> confirm` is the next command and this one
//! prints it.
//!
//! **It does not verify that the certificate and the key go together.** That
//! check needs the private half of an ECDSA pair and belongs where it already
//! is: every consumer refuses at start-up, by name, before serving a byte —
//! `rivet-proxyd` and `rivet_sdk::serve::Tls` both. What this refuses is the
//! mistake that gets past that: a file that is not what its name says.

use std::path::{Path, PathBuf};

use rivet_remote::endpoint::{Endpoint, IDENTITY_AND_PIN};

/// The three names, and they are the fleet's rather than this tool's.
///
/// `rivet-proxyd`, `tempmon-rivet`, `trends-rivet` and
/// `aura-controller` all look for exactly these words, so **an operator
/// installing three secrets into five applications types the same name each
/// time**. They are spelled again here rather than imported because four of
/// those five are foreign programs that link nothing of Rivet's (`ADR-036` §5),
/// so there is no crate to import them from — the agreement is a convention and
/// this comment is where it is written down.
///
/// **Since `ADR-055` §3 a manifest can DECLARE that it needs them**, as the
/// kind `fleet-tls`, and `rivet_core::package::application::Material` carries
/// the same three names for the deploy-time refusal. That is a second spelling
/// of one fact, so a test holds the two lists side by side
/// (`the_kinds_files_are_exactly_what_cargo_rivet_secrets_installs`) — a
/// refusal naming files this verb does not write would send an operator after
/// the wrong thing.
pub(crate) const CERTIFICATE: &str = "tls-cert";
/// Its private key.
pub(crate) const KEY: &str = "tls-key";
/// The authority a caller must chain to.
pub(crate) const AUTHORITY: &str = "tls-root";

/// The authority's own certificate, inside the authority directory.
const ROOT_FILE: &str = "root.crt";

/// What a server pair's certificate is called: `server-<name>.crt`.
const SERVER_PREFIX: &str = "server-";

/// What `secrets` was asked to do.
#[derive(Debug)]
pub struct SecretsArgs {
    /// The application whose data directory the three files go into.
    pub application: String,
    /// Where the authority lives.
    pub authority: PathBuf,
    /// The server pair to install, by the name in its filename. `None` means
    /// *there had better be exactly one*.
    pub name: Option<String>,
    pub endpoint: Endpoint,
    /// Whether to act. Without it, the plan.
    pub confirm: bool,
}

/// `$HOME/.rivet/ca`, which is where `rivet-provision` puts an authority.
///
/// **The same rule [`crate::credentials`] uses** and for its reason: `~/.rivet`
/// is a directory this project's tools already own, and a person looking for a
/// key will look there.
fn default_authority() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".rivet").join("ca"))
}

/// **`<device> <application>`: two bare words, and the first names a device.**
///
/// One bare word is an application alone, the grammar this verb always had,
/// and gives `None`. With two, the first is a name in `rivet-fleet`'s register
/// that `crate::device` resolves into `--host`, `--identity` and `--pin`,
/// exactly as `run <device>` and `install <device>` are resolved — so the
/// command a deploy prints carries no paths for a shell to mangle. Returns the
/// device, the application, and the words after them.
pub(crate) fn leading_device(args: &[String]) -> Option<(&str, &str, &[String])> {
    let bare = |word: &String| !word.starts_with('-') && word != "confirm";
    match args {
        [device, application, rest @ ..] if bare(device) && bare(application) => {
            Some((device.as_str(), application.as_str(), rest))
        }
        _ => None,
    }
}

/// How to say it.
pub fn usage() -> String {
    format!(
        "cargo rivet secrets [DEVICE] APPLICATION [--host HOST:PORT --identity DIR --pin HEX] \
         [--authority DIR] [--name NAME] [confirm]\n\
         \n\
         Install the three TLS files an application needs to serve HTTPS behind\n\
         `rivet-proxyd`, and to call another application that requires one:\n\
         \n\
        \x20 {CERTIFICATE}   the certificate this machine serves — `{SERVER_PREFIX}<name>.crt`\n\
        \x20 {KEY}    its private key — `{SERVER_PREFIX}<name>.key`\n\
        \x20 {AUTHORITY}   the fleet authority a caller must chain to — `{ROOT_FILE}`\n\
         \n\
         Without `confirm` it prints the plan and sends nothing.\n\
         \n\
        \x20 DEVICE           a name in rivet-fleet's register, before the\n\
        \x20                  application. It supplies --host, --identity and\n\
        \x20                  --pin, exactly as `cargo rivet run DEVICE` does;\n\
        \x20                  without it, give all three.\n\
        \x20 --authority DIR  where the authority lives. Default `~/.rivet/ca`.\n\
        \x20 --name NAME      which server pair, when the directory holds more\n\
        \x20                  than one. `rivet-01.local`, not a file name.\n\
         \n\
         It does not restart the application. An application reads these at\n\
         start, so stopping one that is serving is the operator's call:\n\
        \x20 cargo rivet run … -- apply service-restart APPLICATION confirm\n"
    )
}

/// Read the words.
///
/// # Errors
///
/// A missing application, a flag with no value, a word this verb does not take,
/// or either half of `ADR-025` §2's trust without the other.
pub fn parse(args: &[String]) -> Result<SecretsArgs, String> {
    let mut application: Option<String> = None;
    let mut authority: Option<PathBuf> = None;
    let mut name: Option<String> = None;
    let mut endpoint = Endpoint::default();
    let mut confirm = false;

    let mut index = 0;
    while index < args.len() {
        if let Some(next) = endpoint.read(args, index)? {
            index = next;
            continue;
        }
        match args[index].as_str() {
            "--authority" => {
                authority = Some(PathBuf::from(value_after(args, index, "--authority")?));
                index += 2;
            }
            "--name" => {
                name = Some(value_after(args, index, "--name")?);
                index += 2;
            }
            "confirm" => {
                confirm = true;
                index += 1;
            }
            other if other.starts_with('-') => {
                return Err(format!("`{other}` is not a flag this verb takes.\n\n{}", usage()));
            }
            other if application.is_none() => {
                application = Some(other.to_owned());
                index += 1;
            }
            other => {
                return Err(format!(
                    "`{other}`: this verb installs into ONE application at a time, and \
                     `{}` was already named.\n\n{}",
                    application.unwrap_or_default(),
                    usage()
                ));
            }
        }
    }

    let application = application.ok_or_else(|| {
        format!("an application to install into is required.\n\n{}", usage())
    })?;
    endpoint.paired().map_err(|e| format!("{e}\n\n{IDENTITY_AND_PIN}"))?;
    let authority = match authority {
        Some(directory) => directory,
        None => default_authority().ok_or_else(|| {
            "no HOME or USERPROFILE, so there is no default authority directory — name one \
             with --authority"
                .to_owned()
        })?,
    };

    Ok(SecretsArgs {
        application,
        authority,
        name,
        endpoint,
        confirm,
    })
}

/// A flag's value, or a sentence naming the flag.
fn value_after(args: &[String], index: usize, flag: &str) -> Result<String, String> {
    args.get(index + 1)
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value.\n\n{}", usage()))
}

/// One file, as it will be installed.
struct Piece {
    /// What it is called in the application's data directory.
    secret: &'static str,
    /// Where it came from.
    source: PathBuf,
    /// Its bytes.
    content: Vec<u8>,
}

impl core::fmt::Debug for Piece {
    /// **Redacted, and hand-written for exactly that reason.**
    ///
    /// One of the three of these is a private key. A derived `Debug` would
    /// print it the first time anybody put a `{:?}` in a diagnostic — and the
    /// first caller was a test's `expect_err`, which is how this was found.
    /// The length is the useful part and it is the part that is safe.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "Piece {{ secret: {:?}, source: {:?}, content: <{} bytes, redacted> }}",
            self.secret,
            self.source,
            self.content.len()
        )
    }
}

/// Do it, or say what would be done.
///
/// # Errors
///
/// A missing or unusable file, an authority directory with no server pair or
/// with several and no `--name` to choose between them, or the machine's own
/// refusal of any one of the three calls.
pub fn run(args: &SecretsArgs) -> Result<String, String> {
    let (name, pieces) = gather(args)?;

    if !args.confirm {
        return Ok(plan(args, &name, &pieces));
    }
    if args.endpoint.host.is_none() {
        return Err(format!(
            "--host is required to install anything. Without it this verb can print a plan \
             and nothing else.\n\n{}",
            usage()
        ));
    }

    let mut report = String::new();
    for piece in &pieces {
        // **Built as a word vector and handed to `run`, never to a shell.**
        // One of these carries a private key, and a command line is visible in
        // the process table and lands in a shell's history.
        let hex = rivet_core::package::key_material::to_hex(&piece.content);
        let mut words: Vec<String> = Vec::new();
        if let Some(host) = &args.endpoint.host {
            words.push("--host".to_owned());
            words.push(host.clone());
        }
        if let Some(identity) = &args.endpoint.identity {
            words.push("--identity".to_owned());
            words.push(identity.display().to_string());
        }
        if let Some(pin) = &args.endpoint.pin {
            words.push("--pin".to_owned());
            words.push(pin.clone());
        }
        words.extend(
            [
                "--",
                "apply",
                "install-secret",
                &args.application,
                piece.secret,
                "--hex",
                &hex,
                "confirm",
            ]
            .map(str::to_owned),
        );

        let answer = crate::run::execute(&words).map_err(|e| {
            format!(
                "{}: {e}\n\nWhat was installed before this one is still installed; nothing \
                 rolls back. Run the same command again once the cause is fixed — an install \
                 replaces what is there.",
                piece.secret
            )
        })?;
        report.push_str(&read_answer(&answer, args, piece)?);
    }
    report.push_str(&format!(
        "\nAll three are in {}'s data directory, owned by it, mode 0600.\n\
         \n\
         NOTHING WAS RESTARTED, and an application reads these at start — so\n\
         until it does, this has changed nothing it can see:\n\
        \x20 cargo rivet run --host {} --identity DIR --pin HEX -- \\\n\
        \x20     apply service-restart {} confirm\n",
        args.application,
        args.endpoint.host.as_deref().unwrap_or("HOST:PORT"),
        args.application,
    ));
    Ok(report)
}

/// One line out of the machine's answer, and three checks on the way.
///
/// **The answer is checked to be about the question** — `crate::screenshot`
/// does the same and for the same reason: one connection carries several calls,
/// and an answer bound to the wrong one would report a success about a file
/// nobody named.
///
/// The byte count is the check worth having. It is what the machine **decoded**,
/// so comparing it against what was read here proves the hex survived the trip
/// — a truncated credential installs cleanly and refuses at start-up with a
/// message about parsing, which is the wrong end to find that out.
///
/// The answer is summarised rather than printed. It carries three paragraphs of
/// correct and important prose about attribution and about nothing having been
/// restarted; **three copies of it is a wall nobody reads**, and the parts that
/// differ between them are the four fields below.
fn read_answer(answer: &str, args: &SecretsArgs, piece: &Piece) -> Result<String, String> {
    use rivet_core::json::Json;

    let json = rivet_core::json::parse(answer).map_err(|why| {
        format!(
            "{}: the machine's answer is not JSON this build can read ({why}). It said:\n\
             {answer}",
            piece.secret
        )
    })?;
    let string = |key: &str| json.get(key).and_then(Json::as_str).map(str::to_owned);
    let integer = |key: &str| json.get(key).and_then(Json::as_integer);

    match string("application").as_deref() {
        Some(named) if named == args.application => {}
        Some(named) => {
            return Err(format!(
                "{}: this asked about `{}` and the machine answered about `{named}`. \
                 Nothing is claimed about either.",
                piece.secret, args.application
            ));
        }
        None => {
            return Err(format!(
                "{}: the machine's answer names no application, so this build cannot tell \
                 what it is about. It said:\n{answer}",
                piece.secret
            ));
        }
    }

    let wrote = integer("bytes").ok_or_else(|| {
        format!(
            "{}: the machine's answer carries no byte count, so nothing here can say the \
             credential arrived whole. It said:\n{answer}",
            piece.secret
        )
    })?;
    let sent = i64::try_from(piece.content.len()).unwrap_or(i64::MAX);
    if wrote != sent {
        return Err(format!(
            "{}: {sent} bytes were sent and the machine decoded {wrote}. The credential on \
             that machine is NOT the file on this one; do not restart the application until \
             this is understood.",
            piece.secret
        ));
    }

    let path = string("path").unwrap_or_else(|| "(unstated)".to_owned());
    let replaced = integer("replaced-bytes")
        .map_or_else(String::new, |old| format!("  (replaced {old} bytes)"));
    Ok(format!(
        "\x20 {:<9} {wrote:>5} bytes  →  {path}{replaced}\n",
        piece.secret
    ))
}

/// The plan, which is what this prints without `confirm`.
///
/// **It names lengths and never content.** One of the three is a private key,
/// and a plan is rendered to a terminal, returned over a control plane and read
/// by an agent whose transcript outlives the session —
/// `rivet_core::operation::install_secret` learned that from a test and this
/// file is not going to relearn it.
fn plan(args: &SecretsArgs, name: &str, pieces: &[Piece]) -> String {
    let mut out = format!(
        "Install three TLS files into `{}` on {}:\n\n",
        args.application,
        args.endpoint.host.as_deref().unwrap_or("a machine not yet named"),
    );
    for piece in pieces {
        out.push_str(&format!(
            "\x20 {:<9} {:>5} bytes  ←  {}\n",
            piece.secret,
            piece.content.len(),
            piece.source.display()
        ));
    }
    // **Derived, never spelled.** `rivet_core::settings::path` is where the
    // supervisor writes the file, and a path written out here would keep naming
    // the old place the day that moves.
    let proxy_settings =
        rivet_core::service::ServiceName::new(rivet_core::service::PROXYD_SERVICE_NAME).map_or_else(
            || "the proxy's settings file".to_owned(),
            |proxy| rivet_core::settings::path(rivet_core::service::BUILT_IN_VOLUME, &proxy),
        );
    out.push_str(&format!(
        "\nEach lands at /state/app/{}/data/<name>, mode 0600, owned by that\n\
         application. An existing one is REPLACED and the old bytes are gone.\n\
         \n\
         The certificate is for `{name}`. That has to be the name `rivet-proxyd`\n\
         verifies its backends against — its `upstream-name` setting — or every\n\
         proxied request fails the handshake rather than the route (ADR-046 §4.2).\n\
         Read it with:\n\
        \x20 cargo rivet run … -- get file {proxy_settings}\n\
         \n\
         Nothing was sent. Add `confirm`.\n",
        args.application
    ));
    out
}

/// Find and read the three files.
fn gather(args: &SecretsArgs) -> Result<(String, Vec<Piece>), String> {
    if !args.authority.is_dir() {
        return Err(format!(
            "{}: there is no authority directory here.\n\
            \x20 why  this verb installs an authority's files and does not mint any. \
             `rivet-provision ca <suffix> <fleet-name> <dir>` makes one, and \
             `rivet-provision ca server <hostname>` mints the pair a machine serves.",
            args.authority.display()
        ));
    }
    let (name, certificate, key) = server_pair(args)?;
    let root = args.authority.join(ROOT_FILE);

    let pieces = vec![
        Piece {
            secret: CERTIFICATE,
            content: read_pem(&certificate, Kind::Certificate)?,
            source: certificate,
        },
        Piece {
            secret: KEY,
            content: read_pem(&key, Kind::PrivateKey)?,
            source: key,
        },
        Piece {
            secret: AUTHORITY,
            content: read_pem(&root, Kind::Certificate)?,
            source: root,
        },
    ];
    Ok((name, pieces))
}

/// Which server pair, and where its two halves are.
///
/// **One in the directory is chosen; several is a question rather than a
/// guess.** A fleet with two machines has two pairs here, and installing the
/// wrong one produces a handshake failure whose message is about a name nobody
/// typed.
fn server_pair(args: &SecretsArgs) -> Result<(String, PathBuf, PathBuf), String> {
    if let Some(name) = &args.name {
        let certificate = args.authority.join(format!("{SERVER_PREFIX}{name}.crt"));
        let key = args.authority.join(format!("{SERVER_PREFIX}{name}.key"));
        if !certificate.is_file() {
            return Err(format!(
                "{}: no certificate for {name} here.\n\
                \x20 why  --name takes a machine's fleet name — `rivet-01.local` — and this \
                 verb looks for `{SERVER_PREFIX}<name>.crt` beside the authority.\n\
                \x20 make one  rivet-provision ca server {name}",
                certificate.display()
            ));
        }
        return Ok((name.clone(), certificate, key));
    }

    let mut found: Vec<String> = Vec::new();
    let entries = std::fs::read_dir(&args.authority)
        .map_err(|e| format!("{}: {e}", args.authority.display()))?;
    for entry in entries.flatten() {
        let file = entry.file_name();
        let Some(file) = file.to_str() else { continue };
        if let Some(rest) = file.strip_prefix(SERVER_PREFIX)
            && let Some(name) = rest.strip_suffix(".crt")
        {
            found.push(name.to_owned());
        }
    }
    found.sort();

    match found.as_slice() {
        [] => Err(format!(
            "{}: this authority has no server certificate.\n\
            \x20 why  a backend serves the machine's own certificate, and one is minted per \
             machine once its hostname is known (ADR-046 §4.2).\n\
            \x20 make one  rivet-provision ca server <hostname>",
            args.authority.display()
        )),
        [only] => Ok((
            only.clone(),
            args.authority.join(format!("{SERVER_PREFIX}{only}.crt")),
            args.authority.join(format!("{SERVER_PREFIX}{only}.key")),
        )),
        several => Err(format!(
            "{}: this authority has {} server certificates and nothing said which:\n\
            \x20 {}\n\
             Name one with --name. Installing another machine's certificate is a handshake \
             failure whose message is about a name nobody typed.",
            args.authority.display(),
            several.len(),
            several.join("\n  ")
        )),
    }
}

/// What a file is supposed to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Certificate,
    PrivateKey,
}

impl Kind {
    /// The PEM label this kind opens with.
    const fn labels(self) -> &'static [&'static str] {
        match self {
            Self::Certificate => &["-----BEGIN CERTIFICATE-----"],
            // Both spellings: `rcgen` writes PKCS#8, and a key that came from
            // somewhere else may be the older form. Either is something rustls
            // reads.
            Self::PrivateKey => &[
                "-----BEGIN PRIVATE KEY-----",
                "-----BEGIN EC PRIVATE KEY-----",
                "-----BEGIN RSA PRIVATE KEY-----",
            ],
        }
    }

    const fn describe(self) -> &'static str {
        match self {
            Self::Certificate => "a certificate",
            Self::PrivateKey => "a private key",
        }
    }
}

/// Read a file and check it is the kind of PEM it is supposed to be.
///
/// **The check this makes is the mistake that gets past everything else**: a
/// certificate installed as `tls-key`, or a DER file where PEM was expected.
/// The consumer's own start-up refusal catches a key that does not go with its
/// certificate; nothing catches a swap that early, because both files parse.
fn read_pem(path: &Path, kind: Kind) -> Result<Vec<u8>, String> {
    let content = std::fs::read(path).map_err(|e| {
        format!(
            "{}: {e}\n\
            \x20 why  this is {} the application needs, and this verb installs what is on \
             this host rather than making one.",
            path.display(),
            kind.describe()
        )
    })?;
    if content.is_empty() {
        return Err(format!(
            "{}: empty. An empty secret installs cleanly and refuses at start-up, which is \
             the wrong end to find out.",
            path.display()
        ));
    }
    let head = String::from_utf8_lossy(&content[..content.len().min(4096)]);
    if !kind.labels().iter().any(|label| head.contains(label)) {
        return Err(format!(
            "{}: this does not look like {} in PEM.\n\
            \x20 wanted  a file containing `{}`\n\
            \x20 why  the two halves of a pair are one letter apart in the filename, and a \
             certificate installed as the key is a service that refuses to start with a \
             message about neither file.",
            path.display(),
            kind.describe(),
            kind.labels()[0]
        ));
    }
    Ok(content)
}

#[cfg(test)]
#[path = "secrets_tests.rs"]
mod tests;
