//! Tests for `cargo rivet fleet sync` — the grammar, and the sync against an
//! in-memory repository store standing in for the machine.
//!
//! **What stands in for the machine is the store's three routes, and nothing
//! else.** The machine's side — the door, the kind, the prune rule reading a
//! real git repository — is `cmd/rivet-mcpd/src/repository_tests.rs`'s, and
//! `tools/qemu/adr-052-objects-proof.sh` runs both halves on a booted guest.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use rivet_core::repository_objects::Held;

use super::*;

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|w| (*w).to_owned()).collect()
}

fn scratch(label: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    crate::scratch::absent(&format!(
        "fleet-sync-test-{label}-{}",
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn hex_of(bytes: &[u8]) -> String {
    use rivet_core::package::port::Digest as _;
    crate::key::hex(&rivet_crypto::Blake3::new().of(bytes))
}

/// A scratch fleet repository with a manifest and objects, committed on `main`.
struct Repo(PathBuf);

impl Repo {
    fn new(label: &str) -> Self {
        let root = scratch(label);
        super::super::init(&root).expect("fleet init");
        Self(root)
    }

    /// Name an object of `kind`, and hold its bytes unless `held_here` is false.
    ///
    /// **An `apk` is `arch = "any"`, `platform = "android"`** — `ADR-061`
    /// §3.1's header shape, and what `fleet_publish_apk.rs` actually writes.
    /// Composed here exactly as `cmd/rivet-mcpd/src/repository_tests.rs`'s
    /// `a_trusted_apk_is_verified_and_held` names the same artifact on the
    /// machine side: the two tests together prove one round trip — this end
    /// composes the ADR-027 name, that one drives the real door and holds it.
    fn add(&self, product: &str, version: &str, kind: &str, held_here: bool) -> String {
        let bytes = format!("{product} {version} {kind}").into_bytes();
        let hex = hex_of(&bytes);
        if held_here {
            std::fs::write(self.0.join("objects").join(&hex), &bytes).expect("an object");
        }
        let manifest = self.0.join("manifest.toml");
        let mut text = std::fs::read_to_string(&manifest).unwrap_or_default();
        let (arch, platform) = match kind {
            "update" | "card" => ("aarch64", "platform = \"rpi4\"\n"),
            "apk" => ("any", "platform = \"android\"\n"),
            _ => ("aarch64", ""),
        };
        text.push_str(&format!(
            "\n[[artifact]]\ndigest = \"{hex}\"\nproduct = \"{product}\"\nversion = \"{version}\"\n\
             arch = \"{arch}\"\n{platform}kind = \"{kind}\"\nsize = {}\nadded = \"2026-09-14\"\n",
            bytes.len()
        ));
        std::fs::write(&manifest, text).expect("the manifest");
        hex
    }

    /// Withhold `product` of `kind` from every fleet, in the shape
    /// `fleet::render_manifest` writes.
    fn hide(&self, product: &str, kind: &str) {
        let manifest = self.0.join("manifest.toml");
        let mut text = std::fs::read_to_string(&manifest).unwrap_or_default();
        text.push_str(&format!("\n[[hidden]]\nproduct = \"{product}\"\nkind    = \"{kind}\"\n"));
        std::fs::write(&manifest, text).expect("the manifest");
    }

    /// Remove an entry from the manifest by digest.
    fn drop_entry(&self, digest: &str) {
        let manifest = self.0.join("manifest.toml");
        let text = std::fs::read_to_string(&manifest).expect("the manifest");
        let kept: Vec<&str> = text
            .split("\n[[artifact]]\n")
            .filter(|block| !block.contains(digest))
            .collect();
        std::fs::write(&manifest, kept.join("\n[[artifact]]\n")).expect("the manifest");
    }

    fn commit(&self) -> String {
        super::super::git(&self.0, &["add", "-A"]).expect("add");
        super::super::git(&self.0, &["commit", "-q", "--allow-empty", "-m", "manifest"]).expect("commit");
        super::super::git(&self.0, &["rev-parse", "HEAD"]).expect("rev-parse").trim().to_owned()
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// **A repository store, in memory**, answering as the machine's does: what
/// its manifest names follows `rivet_core::repository_objects::named`, and its
/// prune follows `rivet_core::repository_objects::Listing::prune`.
#[derive(Default)]
struct FakeStore {
    held: RefCell<BTreeMap<String, String>>,
    /// The server's own manifest: its commit and what it names.
    manifest: RefCell<Option<(String, Vec<String>)>>,
    puts: RefCell<Vec<String>>,
    prunes: RefCell<usize>,
    fail: RefCell<Vec<String>>,
}

impl FakeStore {
    fn listing(&self) -> Listing {
        let manifest = match &*self.manifest.borrow() {
            Some((commit, named)) => Manifest::Read {
                commit: commit.clone(),
                named: named.iter().map(|d| Digest::new(d).expect("digest")).collect(),
            },
            None => Manifest::Unavailable("no main yet".to_owned()),
        };
        let held = self
            .held
            .borrow()
            .iter()
            .map(|(d, n)| Held { digest: Digest::new(d).expect("digest"), name: n.clone() })
            .collect();
        Listing { manifest, held }
    }

    /// The server's manifest becomes `repo`'s, as a push would make it, named
    /// by the server's own rule — `rivet_core::repository_objects::named`,
    /// hidden set included.
    fn pushed(&self, repo: &Repo, commit: &str) {
        let entries = super::super::read_manifest(&repo.0).expect("reads");
        let hidden = super::super::read_hidden(&repo.0).expect("reads");
        let hidden: Vec<(&str, &str)> = hidden.iter().map(|h| (h.product.as_str(), h.kind.as_str())).collect();
        let named: Vec<String> = objects::named(
            entries.iter().map(|e| (e.digest.as_str(), e.product.as_str(), e.kind.as_str())),
            &hidden,
        )
        .expect("names")
        .into_iter()
        .map(|d| d.as_str().to_owned())
        .collect();
        *self.manifest.borrow_mut() = Some((commit.to_owned(), named));
    }
}

impl ObjectStore for FakeStore {
    fn list(&self) -> Result<String, String> {
        Ok(self.listing().render())
    }

    fn put(&self, local: &Path, name: &str) -> Result<String, String> {
        self.puts.borrow_mut().push(name.to_owned());
        if self.fail.borrow().iter().any(|n| n == name) {
            return Err("the transfer failed after 0 of 9 bytes: reset".to_owned());
        }
        if name.starts_with("tampered-") {
            return Err(format!(
                "rivet REFUSED THE ARTIFACT {name}.\n  why  member 1 does not match its digest"
            ));
        }
        let bytes = std::fs::read(local).map_err(|e| e.to_string())?;
        self.held.borrow_mut().insert(hex_of(&bytes), name.to_owned());
        Ok("201".to_owned())
    }

    fn prune(&self) -> Result<String, String> {
        *self.prunes.borrow_mut() += 1;
        let remove = self.listing().prune()?;
        let mut body = String::new();
        for one in remove {
            self.held.borrow_mut().remove(one.digest.as_str());
            body.push_str(&format!("removed {} {}\n", one.digest, one.name));
        }
        Ok(body)
    }
}

// ---------------------------------------------------------------------------
// the grammar
// ---------------------------------------------------------------------------

#[test]
fn sync_reads_a_device_prune_and_confirm_and_push_takes_no_prune() {
    match super::super::parse(&words(&["sync", "rivet-01", "--prune", "--at", "./f", "confirm"])).expect("parses") {
        super::super::FleetArgs { at, verb: super::super::SubVerb::Sync { device, prune, confirm, .. } } => {
            assert_eq!(device.as_deref(), Some("rivet-01"));
            assert!(prune && confirm);
            assert_eq!(at, Some(PathBuf::from("./f")));
        }
        _ => panic!("did not parse as a sync"),
    }
    let refused = super::super::parse(&words(&["push", "rivet-01", "--prune"])).err().expect("push takes no --prune");
    assert!(refused.contains("--prune"), "{refused}");
}

// ---------------------------------------------------------------------------
// the sync
// ---------------------------------------------------------------------------

/// **A plan sends nothing**, and names what it would send.
#[test]
fn a_plan_sends_nothing_and_says_what_it_would() {
    let repo = Repo::new("plan");
    repo.add("hello", "0.1.0", "application", true);
    let commit = repo.commit();
    let store = FakeStore::default();
    store.pushed(&repo, &commit);
    let report = sync(&repo.0, &store, "rivet-01", false, false).expect("a plan");
    assert!(report.contains("would send"), "{report}");
    assert!(report.contains("Nothing was sent"), "{report}");
    assert!(store.puts.borrow().is_empty(), "a plan sent {:?}", store.puts.borrow());
}

/// **Only the difference is sent, and a rerun sends nothing** — resumable by
/// construction. Every kind the store holds is sent: an apk (`ADR-061` §3),
/// and a system update and a card (`ADR-063` §6).
#[test]
fn only_the_difference_is_sent_and_a_rerun_sends_nothing() {
    let repo = Repo::new("difference");
    let one = repo.add("hello", "0.1.0", "application", true);
    repo.add("libfoo", "1.0.0", "library", true);
    repo.add("gantry", "0.1.17", "apk", true);
    repo.add("rivet", "0.1.0", "update", true);
    repo.add("rivet", "0.1.0", "card", true);
    let commit = repo.commit();
    let store = FakeStore::default();
    store.pushed(&repo, &commit);
    store.held.borrow_mut().insert(one.clone(), "hello-0.1.0-aarch64-application.rvt".to_owned());

    let report = sync(&repo.0, &store, "rivet-01", false, true).expect("synced");
    assert_eq!(
        *store.puts.borrow(),
        vec![
            "libfoo-1.0.0-aarch64-library.rvt".to_owned(),
            "gantry-0.1.17-any-android-apk.rvt".to_owned(),
            "rivet-0.1.0-aarch64-rpi4-update.rvt".to_owned(),
            "rivet-0.1.0-aarch64-rpi4-card.rvt".to_owned(),
        ],
        "{report}"
    );
    // A held object is counted, never listed one per line.
    assert!(!report.contains(&one), "{report}");
    assert!(report.contains("1 object(s) the server holds, not sent"), "{report}");
    assert!(report.contains("5 object(s): applications, libraries, apks, updates and cards"), "{report}");
    assert!(report.contains("SYNCED"), "{report}");

    store.puts.borrow_mut().clear();
    let again = sync(&repo.0, &store, "rivet-01", false, true).expect("synced again");
    assert!(store.puts.borrow().is_empty(), "a rerun sent {:?}", store.puts.borrow());
    assert!(again.contains("0 sent, 5 already held"), "{again}");
}

/// **The repository's keep-three reaches the server's store for cards**
/// (`ADR-063` §6) — the two halves together: the server holds four cards, the
/// workstation's manifest names the newest three, and a sync with `--prune`
/// leaves exactly those three.
#[test]
fn a_prune_after_sync_leaves_the_three_cards_the_manifest_keeps() {
    let repo = Repo::new("card-retention");
    let oldest = repo.add("rivet", "0.1.0", "card", true);
    let kept: Vec<String> =
        ["0.1.1", "0.1.2", "0.1.3"].iter().map(|v| repo.add("rivet", v, "card", true)).collect();
    let store = FakeStore::default();
    store.held.borrow_mut().insert(oldest.clone(), "rivet-0.1.0-aarch64-rpi4-card.rvt".to_owned());
    repo.drop_entry(&oldest);
    let commit = repo.commit();
    store.pushed(&repo, &commit);

    let report = sync(&repo.0, &store, "rivet-01", true, true).expect("synced and pruned");
    assert!(report.contains(&format!("pruned    {oldest}")), "{report}");
    let mut held: Vec<String> = store.held.borrow().keys().cloned().collect();
    held.sort();
    let mut want = kept.clone();
    want.sort();
    assert_eq!(held, want, "{report}");
}

/// **An application the image provides is not sent, and the report says so**
/// — `ADR-060` §4. A repository published before the proxy moved into the
/// image still names proxy payloads; the machine's door would refuse each one.
#[test]
fn an_application_the_image_provides_is_skipped_and_named() {
    let repo = Repo::new("image-service");
    let proxy = repo.add("rivet-proxyd", "0.1.113", "application", true);
    repo.add("hello", "0.1.0", "application", true);
    let commit = repo.commit();
    let store = FakeStore::default();
    store.pushed(&repo, &commit);

    let report = sync(&repo.0, &store, "rivet-01", false, true).expect("synced");
    assert_eq!(
        *store.puts.borrow(),
        vec!["hello-0.1.0-aarch64-application.rvt".to_owned()],
        "{report}"
    );
    assert!(report.contains(&format!("image     {proxy}")), "{report}");
    assert!(report.contains("ADR-060"), "{report}");
    assert!(report.contains("SYNCED"), "{report}");
}

/// **`ADR-052` §7: a hidden product is not sent, the report names it, and the
/// sync is complete without it.** Unhiding it sends it at the next sync.
#[test]
fn a_hidden_product_is_not_sent_and_an_unhidden_one_is() {
    let repo = Repo::new("hidden");
    let testbed = repo.add("testbed", "0.1.3", "apk", true);
    repo.add("hello", "0.1.0", "application", true);
    repo.hide("testbed", "apk");
    let commit = repo.commit();
    let store = FakeStore::default();
    store.pushed(&repo, &commit);

    let report = sync(&repo.0, &store, "rivet-01", false, true).expect("synced");
    assert_eq!(
        *store.puts.borrow(),
        vec!["hello-0.1.0-aarch64-application.rvt".to_owned()],
        "{report}"
    );
    assert!(report.contains(&format!("hidden    {testbed}")), "{report}");
    assert!(report.contains("SYNCED"), "{report}");

    // Unhidden: the manifest without its [[hidden]] entry.
    let manifest = repo.0.join("manifest.toml");
    let text = std::fs::read_to_string(&manifest).expect("the manifest");
    let without = text.replace("\n[[hidden]]\nproduct = \"testbed\"\nkind    = \"apk\"\n", "");
    assert_ne!(text, without, "the hidden entry was not where this test put it");
    std::fs::write(&manifest, without).expect("the manifest");
    let commit = repo.commit();
    store.pushed(&repo, &commit);
    store.puts.borrow_mut().clear();
    sync(&repo.0, &store, "rivet-01", false, true).expect("synced");
    assert_eq!(*store.puts.borrow(), vec!["testbed-0.1.3-any-android-apk.rvt".to_owned()]);
}

/// **An object named here and absent from `objects/` is reported and fails the
/// run**, and the rest are still sent.
#[test]
fn an_object_missing_here_is_reported_and_the_rest_still_sent() {
    let repo = Repo::new("missing");
    repo.add("hello", "0.1.0", "application", true);
    let gone = repo.add("hello", "0.2.0", "application", false);
    let commit = repo.commit();
    let store = FakeStore::default();
    store.pushed(&repo, &commit);
    let report = sync(&repo.0, &store, "rivet-01", false, true).expect_err("incomplete");
    assert!(report.contains(&format!("missing   {gone}")), "{report}");
    assert_eq!(store.puts.borrow().len(), 1, "{report}");
    assert!(report.contains("NOT COMPLETE"), "{report}");
}

/// **A refusal is reported as the ARTIFACT's**, with the server's reason, and
/// a transfer failure as a failure.
#[test]
fn a_refusal_and_a_failure_are_reported_apart() {
    let repo = Repo::new("refused");
    repo.add("hello", "0.1.0", "application", true);
    repo.add("broken", "0.1.0", "application", true);
    repo.add("tampered", "0.1.0", "application", true);
    let commit = repo.commit();
    let store = FakeStore::default();
    store.pushed(&repo, &commit);
    store.fail.borrow_mut().push("broken-0.1.0-aarch64-application.rvt".to_owned());
    let report = sync(&repo.0, &store, "rivet-01", false, true).expect_err("a failure fails the run");
    assert!(report.contains("refused   "), "{report}");
    assert!(report.contains("member 1 does not match its digest"), "{report}");
    assert!(report.contains("failed    "), "{report}");
    assert!(report.contains("1 sent, 0 already held, 0 missing here, 1 refused, 1 failed"), "{report}");
}

/// **`--prune` removes exactly what the server's manifest stops naming**, after
/// a complete sync and a push.
#[test]
fn prune_after_a_complete_sync_removes_exactly_the_dropped_object() {
    let repo = Repo::new("prune");
    let old = repo.add("hello", "0.1.0", "application", true);
    let new = repo.add("hello", "0.2.0", "application", true);
    let commit = repo.commit();
    let store = FakeStore::default();
    store.pushed(&repo, &commit);
    sync(&repo.0, &store, "rivet-01", false, true).expect("synced");
    assert_eq!(store.held.borrow().len(), 2);

    repo.drop_entry(&old);
    let commit = repo.commit();
    store.pushed(&repo, &commit);
    let report = sync(&repo.0, &store, "rivet-01", true, true).expect("pruned");
    assert!(report.contains(&format!("pruned    {old}")), "{report}");
    assert_eq!(store.held.borrow().keys().cloned().collect::<Vec<_>>(), vec![new]);
}

/// **`--prune` is refused while the server's text is behind**, and the server
/// is never asked.
#[test]
fn prune_is_not_run_while_the_servers_manifest_is_not_this_main() {
    let repo = Repo::new("behind");
    let old = repo.add("hello", "0.1.0", "application", true);
    repo.add("hello", "0.2.0", "application", true);
    let commit = repo.commit();
    let store = FakeStore::default();
    store.pushed(&repo, &commit);
    sync(&repo.0, &store, "rivet-01", false, true).expect("synced");

    repo.drop_entry(&old);
    repo.commit();
    // Not pushed: the server still names both.
    let report = sync(&repo.0, &store, "rivet-01", true, true).expect_err("refused");
    assert!(report.contains("PRUNE NOT RUN"), "{report}");
    assert_eq!(*store.prunes.borrow(), 0);
    assert_eq!(store.held.borrow().len(), 2);
}

/// A server whose listing does not read is asked for nothing more.
#[test]
fn a_listing_that_does_not_read_sends_nothing() {
    struct Old;
    impl ObjectStore for Old {
        fn list(&self) -> Result<String, String> {
            Err("the machine answered 405".to_owned())
        }
        fn put(&self, _: &Path, _: &str) -> Result<String, String> {
            panic!("sent after a failed listing")
        }
        fn prune(&self) -> Result<String, String> {
            panic!("pruned after a failed listing")
        }
    }
    let repo = Repo::new("old");
    repo.add("hello", "0.1.0", "application", true);
    repo.commit();
    let why = sync(&repo.0, &Old, "rivet-01", true, true).expect_err("refused");
    assert!(why.contains("nothing was sent"), "{why}");
}
