//! `cargo rivet fleet fetch <fleet>` and `cargo rivet fleet publish FILE
//! --fleet <fleet>` — **the fleet server is the authority for its repository,
//! and a workstation holds a clone that only fetches.** `ADR-063` §6.
//!
//! A fleet is reached through its bookmark (`crate::fleet::bookmark`). Its
//! repository is the server's `fleet` git repository — the text — and its
//! repository store — the objects (`ADR-052` §7). Nothing here writes a
//! workstation repository that anything else reads.
//!
//! # Fetch
//!
//! The clone is `~/.rivet/fleets/<fleet>/`, laid out as any fleet repository
//! is (`manifest.toml` in git, `objects/<digest>` beside it), so `fleet list
//! --at` and `install --artifact --at` read it as they read one.
//!
//! 1. The server's `main` is fetched through `crate::fleet::push`'s loopback
//!    relay and the clone's `main` fast-forwarded to it. **A clone whose `main`
//!    is not an ancestor of the server's is refused and left alone**: a clone
//!    never commits, so one that has history the server lacks was written by
//!    something else, and nothing here decides which side is right.
//! 2. Every object the server holds is read back (`GET
//!    /repository-objects/<digest>`) unless the clone already has it, and kept
//!    only when its bytes hash to its digest. One the clone already has is
//!    hashed again; a copy that does not match is replaced.
//! 3. An object in the clone that the server no longer holds is removed, so
//!    the clone is what the server holds and nothing more.
//!
//! **So two clones can be stale but never contradictory**: each is the
//! server's `main` at some moment, and every object in each hashes to its
//! name. The report ends with the `main` and the object count it now holds.
//!
//! # Publish
//!
//! A publish composes one commit on top of the server's `main` in a scratch
//! repository beside the clone, never in the clone:
//!
//! 1. the scratch repository takes the server's `main` (or is initialised,
//!    when the server's repository has none yet);
//! 2. `crate::fleet::publish_verified` verifies the artifact and records it —
//!    the same verification, retention and commit a workstation repository
//!    applies;
//! 3. with `confirm`, **the object goes first** (`PUT
//!    /repository-objects/<name>`, the server's door verifying it against the
//!    server's own trust store), **then the text** (`crate::fleet::push`, a
//!    fast-forward and never a force) — files, then the list, so a refused
//!    object means the list is never pushed;
//! 4. the scratch repository is removed and the clone fetches.
//!
//! Without `confirm` nothing reaches the server: the plan says whether the
//! server already holds the object and what the push would do.
//!
//! A publish that loses a race — somebody else's publish landed between the
//! read and the push — is refused as not a fast-forward, with the object
//! already held; running it again composes on top of the new `main`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rivet_core::artifact::Digest;
use rivet_core::package::port::PublicKey;

use super::{push, sync};

/// **A fleet server, as fetch and publish reach it** — both `crate::fleet`'s
/// server traits, and one object read back.
///
/// `rivet_remote::tls::Tls` on a real run; a `git http-backend` and an
/// in-memory store in the tests.
pub(super) trait FleetServer: sync::ObjectStore + push::Upstream + Send + Sync {
    /// Read one held object into `into`, kept only when it hashes to `digest`.
    /// Answers the name the server holds it under.
    ///
    /// # Errors
    ///
    /// Not held, not reachable, or not the object asked for (and `into` is
    /// removed).
    fn object(&self, digest: &str, into: &Path) -> Result<String, String>;
}

impl FleetServer for rivet_remote::tls::Tls {
    fn object(&self, digest: &str, into: &Path) -> Result<String, String> {
        self.get_repository_object(digest, into).map(|received| received.name)
    }
}

/// Where the ref the server's `main` is fetched into lives in a clone.
const FETCHED: &str = "refs/remotes/fleet/main";

// ---------------------------------------------------------------------------
// fetch
// ---------------------------------------------------------------------------

/// **Bring `clone` level with `server`** — see the module documentation.
///
/// # Errors
///
/// `clone` exists and is not a git repository (or holds history the server
/// lacks); the server cannot be read; or any object could not be fetched —
/// the report is the error's text, so what did arrive is not lost.
pub(super) fn fetch<T>(clone: &Path, server: &Arc<T>, fleet: &str) -> Result<String, String>
where
    T: FleetServer + 'static,
{
    prepare_clone(clone)?;
    let upstream: Arc<dyn push::Upstream> = Arc::clone(server) as Arc<dyn push::Upstream>;
    let relay = push::Relay::start(upstream, push::REPOSITORY)?;
    let url = relay.url();
    let text = fetch_text(clone, &relay, &url, fleet);
    drop(relay);
    let mut report = format!(
        "fleet fetch  {fleet}\n\
         \x20 from       {fleet}'s server: the `{}` repository and its repository store (ADR-063 §6)\n\
         \x20 into       {}\n",
        push::REPOSITORY,
        clone.display()
    );
    let main = match text {
        Ok((line, main)) => {
            report.push_str(&line);
            main
        }
        Err(why) => return Err(format!("{report}\n{why}")),
    };

    let listing = sync::listing(server.as_ref(), fleet).map_err(|why| format!("{report}\n{why}"))?;
    let objects_dir = clone.join(super::OBJECTS_DIR);
    std::fs::create_dir_all(&objects_dir)
        .map_err(|e| format!("{report}\ncannot create {}: {e}", objects_dir.display()))?;

    let mut failed = 0usize;
    let mut fetched = 0usize;
    let mut kept = 0usize;
    for held in &listing.held {
        let local = objects_dir.join(held.digest.as_str());
        if local.is_file() {
            match super::blake3_of(&local) {
                Ok((digest, _)) if crate::key::hex(&digest) == held.digest.as_str() => {
                    kept += 1;
                    continue;
                }
                _ => {
                    report.push_str(&format!(
                        "  replace   {} {}  (the copy here does not hash to its name)\n",
                        held.digest, held.name
                    ));
                    let _ = std::fs::remove_file(&local);
                }
            }
        }
        let incoming = objects_dir.join(format!(".incoming-{}", held.digest));
        match server.object(held.digest.as_str(), &incoming) {
            Ok(name) => match std::fs::rename(&incoming, &local) {
                Ok(()) => {
                    fetched += 1;
                    report.push_str(&format!("  fetched   {} {name}  (hashed to its digest)\n", held.digest));
                }
                Err(e) => {
                    failed += 1;
                    let _ = std::fs::remove_file(&incoming);
                    report.push_str(&format!("  failed    {} {}: cannot name it: {e}\n", held.digest, held.name));
                }
            },
            Err(why) => {
                failed += 1;
                report.push_str(&format!("  failed    {} {}\n{}", held.digest, held.name, indented(&why)));
            }
        }
    }

    let removed = remove_not_held(&objects_dir, &listing, &mut report);
    let missing = listing.missing();
    if !missing.is_empty() {
        report.push_str(&format!(
            "  server    its manifest names {} object(s) its store does not hold, so this clone \
             cannot have them either: {}\n",
            missing.len(),
            missing.iter().map(Digest::as_str).collect::<Vec<_>>().join(", ")
        ));
    }
    if let rivet_core::repository_objects::Manifest::Read { commit, .. } = &listing.manifest
        && main.as_deref().is_some_and(|m| m != commit)
    {
        report.push_str(&row(
            "note",
            "the server's main moved between reading its text and its store; fetch again for a \
             clone of one moment",
        ));
    }

    let held_now = listing.held.len() - failed;
    report.push_str(&format!(
        "\n{fetched} fetched, {kept} already here and hashed again, {removed} removed, {failed} failed.\n\
         CLONE  {fleet}  main {}  {held_now} object(s), each hashed to its name\n",
        main.as_deref().unwrap_or("(none)"),
    ));
    if failed == 0 { Ok(report) } else { Err(format!("{report}NOT COMPLETE: fetch again.\n")) }
}

/// `clone` as an empty git repository on `main`, or as it already is.
fn prepare_clone(clone: &Path) -> Result<(), String> {
    if clone.join(".git").is_dir() {
        return Ok(());
    }
    if clone.exists() && std::fs::read_dir(clone).map_or(true, |mut e| e.next().is_some()) {
        return Err(format!(
            "{} exists and is not a clone this tool made (it has no .git and is not empty), so \
             nothing was fetched into it",
            clone.display()
        ));
    }
    std::fs::create_dir_all(clone).map_err(|e| format!("cannot create {}: {e}", clone.display()))?;
    super::git(clone, &["init", "--quiet", "--initial-branch=main"])?;
    Ok(())
}

/// The server's `main`, fetched and fast-forwarded to. Answers the report line
/// and the commit, or `None` when the server's repository has no `main` yet.
fn fetch_text(clone: &Path, relay: &push::Relay, url: &str, fleet: &str) -> Result<(String, Option<String>), String> {
    let Some(server_main) = push::remote_main(clone, url, relay, fleet)? else {
        return Ok((
            row("text", "the server's repository has no main yet: nothing published, so nothing to fetch"),
            None,
        ));
    };
    let fetched = push::remote_git(
        clone,
        &["fetch", "--quiet", "--no-tags", url, &format!("+refs/heads/{}:{FETCHED}", push::BRANCH)],
    )
    .map_err(|e| format!("cannot run `git fetch`: {e}"))?;
    if !fetched.status.success() {
        return Err(format!("could not fetch {fleet}'s main.\n{}", push::quoted_output(&fetched)));
    }
    let local = super::git(clone, &["rev-parse", "--verify", "--quiet", "refs/heads/main^{commit}"])
        .map(|s| s.trim().to_owned())
        .ok()
        .filter(|s| !s.is_empty());
    let line = match local {
        None => {
            super::git(clone, &["checkout", "--quiet", "-B", "main", FETCHED])?;
            row("text", &format!("main {server_main}  (the clone's first)"))
        }
        Some(local) if local == server_main => row("text", &format!("main {server_main}  (already here)")),
        Some(local) => {
            let ancestor = std::process::Command::new("git")
                .args(["merge-base", "--is-ancestor", &local, FETCHED])
                .current_dir(clone)
                .output()
                .map_err(|e| format!("cannot run `git merge-base`: {e}"))?;
            if !ancestor.status.success() {
                return Err(format!(
                    "REFUSED: this clone's main {local} is not an ancestor of {fleet}'s main \
                     {server_main}. A clone never commits, so this one holds history the server \
                     lacks and was written by something else; nothing here decides which side is \
                     right, and nothing was changed."
                ));
            }
            super::git(clone, &["merge", "--quiet", "--ff-only", FETCHED])?;
            row("text", &format!("main {local} -> {server_main}  (fast-forward)"))
        }
    };
    Ok((line, Some(server_main)))
}

/// Remove every object file in `objects_dir` the server no longer holds.
/// Only names that are digests are touched.
fn remove_not_held(objects_dir: &Path, listing: &rivet_core::repository_objects::Listing, report: &mut String) -> usize {
    let Ok(entries) = std::fs::read_dir(objects_dir) else {
        return 0;
    };
    let mut removed = 0usize;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(digest) = Digest::new(&name) else {
            continue;
        };
        if listing.holds(&digest) {
            continue;
        }
        if std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
            report.push_str(&format!("  removed   {digest}  (the server no longer holds it)\n"));
        }
    }
    removed
}

// ---------------------------------------------------------------------------
// publish
// ---------------------------------------------------------------------------

/// **Publish `file` to `server`'s repository, then fetch** — see the module
/// documentation. `work` is the scratch repository's path, removed before and
/// after; `clone` is the fleet's clone.
///
/// # Errors
///
/// The artifact does not verify against `trusted`; the server cannot be read;
/// or, with `confirm`, the object is refused (nothing pushed), the push is
/// refused, or the fetch after it fails.
pub(super) fn publish<T>(
    work: &Path,
    clone: &Path,
    server: &Arc<T>,
    fleet: &str,
    file: &Path,
    trusted: &mut dyn FnMut() -> Result<PublicKey, String>,
    confirm: bool,
) -> Result<String, String>
where
    T: FleetServer + 'static,
{
    let _ = std::fs::remove_dir_all(work);
    let outcome = publish_in(work, server, fleet, file, trusted, confirm);
    let _ = std::fs::remove_dir_all(work);
    let report = outcome?;
    if !confirm {
        return Ok(report);
    }
    match fetch(clone, server, fleet) {
        Ok(fetched) => Ok(format!("{report}\n{fetched}")),
        Err(fetched) => Err(format!("{report}\n{fetched}")),
    }
}

fn publish_in<T>(
    work: &Path,
    server: &Arc<T>,
    fleet: &str,
    file: &Path,
    trusted: &mut dyn FnMut() -> Result<PublicKey, String>,
    confirm: bool,
) -> Result<String, String>
where
    T: FleetServer + 'static,
{
    std::fs::create_dir_all(work).map_err(|e| format!("cannot create {}: {e}", work.display()))?;
    let upstream: Arc<dyn push::Upstream> = Arc::clone(server) as Arc<dyn push::Upstream>;
    let relay = push::Relay::start(upstream, push::REPOSITORY)?;
    let url = relay.url();
    let base = push::remote_main(work, &url, &relay, fleet)?;
    match &base {
        Some(_) => {
            super::git(work, &["init", "--quiet", "--initial-branch=main"])?;
            let fetched = push::remote_git(
                work,
                &["fetch", "--quiet", "--no-tags", &url, &format!("+refs/heads/{}:{FETCHED}", push::BRANCH)],
            )
            .map_err(|e| format!("cannot run `git fetch`: {e}"))?;
            if !fetched.status.success() {
                return Err(format!("could not read {fleet}'s main.\n{}", push::quoted_output(&fetched)));
            }
            super::git(work, &["checkout", "--quiet", "-B", "main", FETCHED])?;
            super::ensure_local_identity(work)?;
            std::fs::create_dir_all(work.join(super::OBJECTS_DIR))
                .map_err(|e| format!("cannot create {}: {e}", work.join(super::OBJECTS_DIR).display()))?;
        }
        None => {
            super::init(work)?;
        }
    }
    drop(relay);

    let published = super::publish_verified(work, file, None, trusted)?;
    let entry = super::read_manifest(work)?
        .into_iter()
        .find(|e| e.digest == published.digest)
        .ok_or_else(|| format!("{} was published and its manifest does not name it", published.digest))?;
    let name = entry.file_name()?;
    let listing = sync::listing(server.as_ref(), fleet)?;
    let digest = Digest::new(&published.digest)
        .ok_or_else(|| format!("{} is not a digest", published.digest))?;
    let held = listing.holds(&digest);

    let mut report = format!(
        "fleet publish  {} -> {fleet}\n\
         \x20 composed on   {fleet}'s main {}\n\n{}\n",
        file.display(),
        base.as_deref().unwrap_or("(none: this is the repository's first commit)"),
        published.report.trim_end(),
    );

    if !confirm {
        report.push_str(&format!(
            "\n  would send  {} {name}{}\n  would push  main, one commit on top of {}\n\
             Nothing reached {fleet}. Add `confirm` to publish.\n",
            published.digest,
            if held { "  (the server already holds it: not sent)" } else { "" },
            base.as_deref().unwrap_or("nothing"),
        ));
        return Ok(report);
    }

    if held {
        report.push('\n');
        report.push_str(&row("held", &format!("{} {name}  (the server already holds it)", published.digest)));
    } else {
        let object = work.join(super::OBJECTS_DIR).join(&published.digest);
        match server.put(&object, &name) {
            Ok(answer) => {
                report.push('\n');
                report.push_str(&row("sent", &format!("{} {name}", published.digest)));
                report.push_str(&indented(&answer));
            }
            Err(why) => {
                return Err(format!(
                    "{report}\n  refused   {} {name}\n{}\nNOT PUBLISHED: the object did not reach \
                     {fleet}'s store, so its main was not pushed.\n",
                    published.digest,
                    indented(&why)
                ));
            }
        }
    }
    let upstream: Arc<dyn push::Upstream> = Arc::clone(server) as Arc<dyn push::Upstream>;
    match push::push(work, upstream, fleet, true, false) {
        Ok(pushed) => Ok(format!("{report}\n{pushed}")),
        Err(pushed) => Err(format!(
            "{report}\n{pushed}\nNOT PUBLISHED: the object is in {fleet}'s store and its main does \
             not name it yet. Running the publish again composes on top of the server's main.\n"
        )),
    }
}

/// One report line: a word padded to the report's column, then the rest —
/// `crate::fleet::sync`'s shape.
fn row(word: &str, rest: &str) -> String {
    format!("  {word:<9} {rest}\n")
}

fn indented(text: &str) -> String {
    text.lines().map(|l| format!("            {l}\n")).collect()
}

/// `~/.rivet/fleets/.<fleet>.publishing` — the scratch repository a publish
/// composes in, beside the clone and never inside it.
pub(super) fn work_dir(clone: &Path) -> PathBuf {
    let fleet = clone.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    clone.with_file_name(format!(".{fleet}.publishing"))
}

#[cfg(test)]
#[path = "fleet_remote_tests.rs"]
mod tests;
