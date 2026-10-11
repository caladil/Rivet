//! `rivet-smbd` — SMB file sharing for a Rivet machine.
//!
//! # Rivet implements no protocol here
//!
//! `PLAN.md` §23.5's survey is the authority and its conclusion is the whole
//! shape of this binary: `ixr-smb-server` already carries all 19 of MS-SMB2's
//! commands, dialects 2.0.2 through 3.1.1, NTLMv2, SPNEGO, SMB 3.1.1 pre-auth
//! integrity and AES-128-CMAC signing — and it has been **mounted from Windows
//! 11 25H2 with `RequireSecuritySignature: True`**, which is the proof its
//! signing is right against Microsoft's implementation rather than only against
//! its own tests.
//!
//! So this crate is a **composition root**: it turns arguments into that
//! library's builder, says what it is about to do, and gets out of the way.
//!
//! # What it deliberately does not do yet
//!
//! **It reads no configuration document.** `ADR-007`'s schema has no `[shares]`
//! table, and adding one is not a detail — `PLAN.md` §23.5 item 1 requires the
//! *absence of encryption* to be a stated decision an operator makes, rather
//! than something discovered. Until that is ruled, everything here is named
//! explicitly on the command line and nothing is implied.
//!
//! **It is not a supervised service.** `cmd/rivet-sysd` has no definition for
//! it, so nothing starts it; `ADR-017`'s supervisor is nonetheless the real
//! mitigation named in §23.5 item 3, because upstream's `panic = "abort"` on a
//! client-controlled length is a designed-in denial of service and a supervisor
//! that restarts is the difference between a dropped connection and a dead
//! share.
//!
//! # The traffic is not encrypted, and this says so every time
//!
//! Upstream, verbatim: *"Encryption (AES-CCM/AES-GCM) is intentionally out of
//! scope for v1."* **Contents are on the wire in the clear.** §23.5 item 1
//! wants that stated where an operator will see it, and until there is a
//! document key to state it in, the only honest place is here, on every start.
//!
//! **What signing covers depends on `--user`.** With it, every session is
//! authenticated, the logon must carry an NTLM MIC, and every request must be
//! signed, so a connection cannot be silently tampered with
//! (`vendor/ixr-smb-server/VENDORING.md`, hunks E to G). Without it, every
//! session is anonymous and has no key to sign with, so that traffic is
//! unsigned — and [`traffic_warning`] says so rather than claiming otherwise.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use smb_server::{Access, LocalFsBackend, Share, SmbServer};

/// One `NAME=PATH` share, as the operator wrote it.
#[derive(Debug)]
struct ShareArg {
    name: String,
    path: PathBuf,
}

/// What this process was asked to serve.
#[derive(Debug)]
struct Args {
    listen: SocketAddr,
    shares: Vec<ShareArg>,
    user: Option<String>,
    read_only: bool,
}

/// The environment variable a password arrives in.
///
/// **Never an argument.** `/proc/<pid>/cmdline` is world-readable, so a
/// password on the command line is a password every process on the machine can
/// read — and `rivet get status` would put it in a report. An environment
/// variable is not secret either (`/proc/<pid>/environ` is at least owner-only)
/// and it is the least-bad option available before `ADR-020`'s secret store
/// exists.
const PASSWORD_VAR: &str = "RIVET_SMBD_PASSWORD";

fn usage() -> String {
    format!(
        "rivet-smbd — SMB file sharing (PLAN.md §23.5, §57, §69)\n\
         \n\
         \x20 --listen ADDR        REQUIRED. Where to bind, e.g. 0.0.0.0:445.\n\
         \x20 --share NAME=PATH    REQUIRED, repeatable. A share and the directory it serves.\n\
         \x20 --user NAME          The one account that may connect. Its password comes from\n\
         \x20                      {PASSWORD_VAR}, never from an argument: /proc/<pid>/cmdline\n\
         \x20                      is world-readable. Without this, shares are PUBLIC.\n\
         \x20 --read-only          Serve every share read-only.\n\
         \n\
         There is no default listen address, no default share and no configuration file.\n\
         ADR-007 has no [shares] table yet, and PLAN.md §23.5 item 1 makes adding one a\n\
         ruling rather than a detail — the absence of encryption has to be a decision an\n\
         operator states, not one they discover.\n"
    )
}

fn parse(argv: &[String]) -> Result<Args, String> {
    let mut listen: Option<SocketAddr> = None;
    let mut shares = Vec::new();
    let mut user = None;
    let mut read_only = false;

    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--listen" => {
                i += 1;
                let raw = argv.get(i).ok_or("--listen needs an address")?;
                listen = Some(raw.parse().map_err(|e| {
                    format!("--listen {raw:?} is not an address and port: {e}")
                })?);
            }
            "--share" => {
                i += 1;
                let raw = argv.get(i).ok_or("--share needs NAME=PATH")?;
                let (name, path) = raw.split_once('=').ok_or_else(|| {
                    format!("--share {raw:?} is not NAME=PATH — it needs one `=`")
                })?;
                if name.is_empty() || path.is_empty() {
                    return Err(format!("--share {raw:?} has an empty name or path"));
                }
                shares.push(ShareArg {
                    name: name.to_owned(),
                    path: PathBuf::from(path),
                });
            }
            "--user" => {
                i += 1;
                user = Some(argv.get(i).ok_or("--user needs a name")?.clone());
            }
            "--read-only" => read_only = true,
            "-h" | "--help" => return Err(usage()),
            other => return Err(format!("{other:?} is not an option of rivet-smbd\n\n{}", usage())),
        }
        i += 1;
    }

    Ok(Args {
        listen: listen.ok_or_else(|| format!("--listen is required\n\n{}", usage()))?,
        shares: if shares.is_empty() {
            return Err(format!("at least one --share is required\n\n{}", usage()));
        } else {
            shares
        },
        user,
        read_only,
    })
}

/// Say what is about to happen, before it happens.
///
/// `ADR-012` §7 makes an operation state its consequence; this is not an
/// operation but the obligation is the same, and one line of it is a warning
/// that cannot be turned off.
fn announce(args: &Args) {
    println!("rivet-smbd: listening on {}", args.listen);
    for share in &args.shares {
        println!(
            "  share     {:<16} {}  [{}]",
            share.name,
            share.path.display(),
            if args.read_only { "read-only" } else { "read-write" }
        );
    }
    match &args.user {
        Some(name) => println!("  user      {name} (password from {PASSWORD_VAR})"),
        None => println!("  user      NONE — every share is PUBLIC and needs no credential"),
    }
    for (i, line) in traffic_warning(args.user.as_deref()).iter().enumerate() {
        let label = if i == 0 { "WARNING" } else { "" };
        println!("  {label:<10}{line}");
    }
}

/// **What protects this share's traffic, said on every start.**
///
/// The two shapes this binary serves differ, and the warning says which one
/// this is. With `--user`, every session is authenticated: the logon must
/// carry an NTLM MIC and every request after it must be signed, so a
/// connection cannot be silently altered. Without it, every session is
/// anonymous, and an anonymous session has no key to sign with — so that
/// traffic is unsigned, which an operator must hear rather than discover.
/// Neither is encrypted: the vendored crate puts AES-CCM/GCM out of scope.
fn traffic_warning(user: Option<&str>) -> &'static [&'static str] {
    match user {
        Some(_) => &[
            "every session authenticates with an NTLM MIC and must SIGN every",
            "request, so a connection cannot be silently tampered with. Traffic",
            "is NOT ENCRYPTED: file CONTENTS are on the wire in the clear.",
        ],
        None => &[
            "every share is PUBLIC, and anonymous traffic is UNSIGNED and NOT",
            "ENCRYPTED: an anonymous session has no key to sign with, so anyone",
            "on the path can read and alter what is sent.",
        ],
    }
}

/// Which of `Share`'s builder methods a share should be given, derived from
/// `--user` and `--read-only`.
///
/// `Share`'s `mode` field is private to the vendored crate: once `build()`
/// below calls `.public()`, `.public_read_only()` or `.user()`, nothing in
/// rivet-smbd can read back which one it picked. So the decision is made
/// here instead, where a test can call it directly with no backend, no
/// listener and no async runtime, and `build()` is a thin match over the
/// result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SharePolicy<'a> {
    /// `Share::public()` — no credential needed, reads and writes both.
    Public,
    /// `Share::public_read_only()` — no credential needed, reads only.
    PublicReadOnly,
    /// `Share::user(user, access)` — only `user`, at `access`, may connect.
    User { user: &'a str, access: Access },
}

fn share_policy(user: Option<&str>, read_only: bool) -> SharePolicy<'_> {
    match user {
        None if read_only => SharePolicy::PublicReadOnly,
        None => SharePolicy::Public,
        Some(user) if read_only => SharePolicy::User {
            user,
            access: Access::Read,
        },
        Some(user) => SharePolicy::User {
            user,
            access: Access::ReadWrite,
        },
    }
}

/// Open `path` as a share root, applying `LocalFsBackend`'s own `read_only()`
/// when `read_only` is set.
///
/// This is independent of `share_policy` above. `Share`'s `ShareMode`/`Access`
/// gate CREATE and WRITE at the protocol layer, but nothing in SMB2's
/// SET_INFO or CLOSE handling consults that grant before a rename, a
/// delete-on-close or a truncate. `LocalFsBackend::read_only()` closes that
/// gap directly — its `unlink`, `rename`, `truncate` and `set_times` all
/// refuse on their own — and additionally reports
/// `BackendCapabilities::is_read_only`, which TREE_CONNECT uses to clamp the
/// protocol-level grant to `Access::Read` regardless of `ShareMode`. Applying
/// it here, from the same `--read-only`, is what makes "serve every share
/// read-only" true of every operation rather than only create and write.
fn backend_for(path: &Path, read_only: bool) -> Result<LocalFsBackend, String> {
    let backend =
        LocalFsBackend::new(path).map_err(|e| format!("cannot serve {}: {e}", path.display()))?;
    Ok(if read_only {
        backend.read_only()
    } else {
        backend
    })
}

fn build(args: &Args) -> Result<SmbServer, String> {
    let password = match &args.user {
        Some(_) => Some(std::env::var(PASSWORD_VAR).map_err(|_| {
            format!(
                "--user was given and {PASSWORD_VAR} is not set. A password is never taken \
                 from an argument, because /proc/<pid>/cmdline is world-readable."
            )
        })?),
        None => None,
    };

    let mut builder = SmbServer::builder().listen(args.listen);
    if let (Some(name), Some(secret)) = (&args.user, &password) {
        builder = builder.user(name.as_str(), secret.as_str());
    }

    let policy = share_policy(args.user.as_deref(), args.read_only);

    for want in &args.shares {
        // backend_for() surfaces its refusal with the share's own name
        // attached: "No such file or directory" on its own does not say
        // WHICH share.
        let backend = backend_for(&want.path, args.read_only)
            .map_err(|e| format!("share {:?} {e}", want.name))?;
        let share = Share::new(want.name.as_str(), backend);
        builder = builder.share(match policy {
            SharePolicy::Public => share.public(),
            SharePolicy::PublicReadOnly => share.public_read_only(),
            SharePolicy::User { user, access } => share.user(user, access),
        });
    }

    builder.build().map_err(|e| format!("rivet-smbd: {e}"))
}

#[tokio::main]
async fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse(&argv) {
        Ok(args) => args,
        Err(why) => {
            eprintln!("rivet-smbd: {why}");
            std::process::exit(2);
        }
    };

    announce(&args);

    let server = match build(&args) {
        Ok(server) => server,
        Err(why) => {
            eprintln!("rivet-smbd: {why}");
            std::process::exit(1);
        }
    };

    // Bind BEFORE reporting readiness: a server that announced and then failed
    // to take the port would be a machine reporting a share nobody can reach.
    if let Err(e) = server.bind().await {
        eprintln!("rivet-smbd: cannot bind {}: {e}", args.listen);
        std::process::exit(1);
    }
    println!("rivet-smbd: bound; serving");

    if let Err(e) = server.serve().await {
        eprintln!("rivet-smbd: stopped serving: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn args(words: &[&str]) -> Result<Args, String> {
        parse(&words.iter().map(|w| (*w).to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn a_listen_and_a_share_are_both_required() {
        assert!(args(&[]).is_err());
        assert!(args(&["--listen", "0.0.0.0:445"]).is_err());
        assert!(args(&["--share", "a=/tmp"]).is_err());
        assert!(args(&["--listen", "0.0.0.0:445", "--share", "a=/tmp"]).is_ok());
    }

    /// **No defaults anywhere.** A share nobody named, on a port nobody chose,
    /// is a machine serving files its operator did not ask it to.
    #[test]
    fn nothing_is_defaulted_and_the_refusal_says_why() {
        let err = args(&[]).expect_err("must refuse");
        assert!(err.contains("--listen is required"), "{err}");
        assert!(err.contains("no default"), "{err}");
    }

    #[test]
    fn a_share_must_be_name_equals_path() {
        for bad in ["public", "=/tmp", "public=", ""] {
            assert!(
                args(&["--listen", "0.0.0.0:445", "--share", bad]).is_err(),
                "{bad:?} should be refused"
            );
        }
        let ok = args(&["--listen", "0.0.0.0:445", "--share", "public=/state/public"])
            .expect("valid");
        assert_eq!(ok.shares[0].name, "public");
        assert_eq!(ok.shares[0].path, PathBuf::from("/state/public"));
    }

    #[test]
    fn several_shares_are_kept_in_the_order_they_were_given() {
        let a = args(&[
            "--listen", "0.0.0.0:445",
            "--share", "one=/a",
            "--share", "two=/b",
        ])
        .expect("valid");
        let names: Vec<&str> = a.shares.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["one", "two"]);
    }

    /// A password never appears among the arguments, so there is no option
    /// that could accept one — asserted so that adding one is a deliberate act.
    #[test]
    fn there_is_no_option_that_takes_a_password() {
        for attempt in ["--password", "--pass", "--secret"] {
            let err = args(&["--listen", "0.0.0.0:445", "--share", "a=/t", attempt, "x"])
                .expect_err("must refuse");
            assert!(err.contains("is not an option"), "{err}");
        }
    }

    #[test]
    fn a_bad_address_is_refused_by_name() {
        let err = args(&["--listen", "not-an-address", "--share", "a=/t"])
            .expect_err("must refuse");
        assert!(err.contains("not-an-address"), "{err}");
    }

    /// **The banner never claims a public share's traffic is signed.** An
    /// anonymous session has no key, so its traffic is unsigned; an
    /// authenticated one must sign. Neither is encrypted.
    #[test]
    fn the_warning_says_anonymous_traffic_is_unsigned_and_authenticated_traffic_signed() {
        let public = traffic_warning(None).join(" ");
        assert!(public.contains("UNSIGNED"), "{public}");
        assert!(public.contains("NOT ENCRYPTED"), "{public}");

        let private = traffic_warning(Some("alice")).join(" ");
        assert!(private.contains("must SIGN every request"), "{private}");
        assert!(!private.contains("UNSIGNED"), "{private}");
        assert!(private.contains("NOT ENCRYPTED"), "{private}");
    }

    /// An anonymous share started with `--read-only` must be handed to
    /// `public_read_only()`, not `public()`. This is the boundary `build()`
    /// cannot be asked directly, because `Share::mode` is private to the
    /// vendored crate — so it is asserted here, at the pure decision.
    #[test]
    fn share_policy_covers_all_four_combinations() {
        assert_eq!(share_policy(None, false), SharePolicy::Public);
        assert_eq!(share_policy(None, true), SharePolicy::PublicReadOnly);
        assert_eq!(
            share_policy(Some("alice"), false),
            SharePolicy::User {
                user: "alice",
                access: Access::ReadWrite
            }
        );
        assert_eq!(
            share_policy(Some("alice"), true),
            SharePolicy::User {
                user: "alice",
                access: Access::Read
            }
        );
    }

    /// `backend_for(_, true)` must make `LocalFsBackend` refuse a write-intent
    /// open, an unlink and a rename on their own — the boundary nothing in
    /// SMB2's SET_INFO/CLOSE handling checks (see `backend_for`'s doc
    /// comment) — while a plain read still works, and `backend_for(_, false)`
    /// must permit the same unlink.
    #[tokio::test]
    async fn a_read_only_backend_refuses_unlink_rename_and_writes_but_not_reads() {
        use smb_server::{OpenOptions, ShareBackend, SmbError, SmbPath};

        let dir = std::env::temp_dir().join(format!("rivet-smbd-read-only-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temporary directory");
        std::fs::write(dir.join("a.txt"), b"hello").expect("a fixture file");

        let from: SmbPath = "a.txt".parse().expect("a valid path");
        let to: SmbPath = "b.txt".parse().expect("a valid path");

        let read_only = backend_for(&dir, true).expect("a read-only backend");
        assert!(read_only.capabilities().is_read_only);

        let write_intent = OpenOptions {
            write: true,
            ..OpenOptions::default()
        };
        assert!(
            matches!(
                read_only.open(&from, write_intent).await,
                Err(SmbError::AccessDenied)
            ),
            "a write-intent open must be refused"
        );
        assert!(
            matches!(read_only.unlink(&from).await, Err(SmbError::AccessDenied)),
            "unlink must be refused"
        );
        assert!(
            matches!(
                read_only.rename(&from, &to).await,
                Err(SmbError::AccessDenied)
            ),
            "rename must be refused"
        );

        // A plain read still works, and sees the untouched fixture — none of
        // the refused calls above mutated anything.
        let handle = read_only
            .open(&from, OpenOptions::default())
            .await
            .expect("a read-only backend still permits a plain read");
        let bytes = handle.read(0, 5).await.expect("the fixture bytes");
        assert_eq!(&bytes[..], b"hello");
        handle.close().await.expect("close");

        // The same unlink succeeds through a read-write backend over the
        // same directory.
        let read_write = backend_for(&dir, false).expect("a read-write backend");
        assert!(!read_write.capabilities().is_read_only);
        read_write
            .unlink(&from)
            .await
            .expect("unlink succeeds when the backend is not read-only");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
