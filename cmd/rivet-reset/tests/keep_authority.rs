//! **`rivet-reset --keep-authority`, run as a program against a scratch
//! workstation.** `SPEC-05` §2.
//!
//! Every test builds a whole `~/.rivet` under a scratch `HOME`, with the
//! registers, the fleet repository and the Armada laid out by the crates that
//! lay out the real ones, and runs the built binary with `HOME` and
//! `USERPROFILE` pointed at it. The operator register is named with
//! `--register`, so nothing resolves `rivet-fleet`'s real configuration.
//!
//! **Provenance of the fixtures:** the register, its projection, the recorded
//! documents and the Armada are written by `rivetos-register` and
//! `rivetos-fleet-repo` themselves. The authority, the seeds, the identity, the
//! notebooks, the manifest and the object are hand-composed bytes: this mode
//! never parses any of them, and what is asserted about them is that they
//! survive byte for byte.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use rivetos_register::armada::{self, Armada};
use rivetos_register::{Device, Register};

/// A scratch `HOME`, removed at the end.
struct Home(PathBuf);

impl Home {
    fn new(label: &str) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rivet-reset-keep-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("a scratch HOME");
        Self(path)
    }

    fn rivet(&self) -> PathBuf {
        self.0.join(".rivet")
    }
    fn ca(&self) -> PathBuf {
        self.rivet().join("ca")
    }
    fn keys(&self) -> PathBuf {
        self.rivet().join("keys")
    }
    fn identity(&self) -> PathBuf {
        self.rivet().join("identity")
    }
    fn provision(&self) -> PathBuf {
        self.rivet().join("provision")
    }
    fn fleet(&self) -> PathBuf {
        self.rivet().join("fleet")
    }
    fn armada(&self) -> PathBuf {
        self.rivet().join("armada")
    }
    /// Where `rivet-fleet` keeps its register on Windows, under this HOME.
    fn register(&self) -> PathBuf {
        self.0.join("AppData").join("Roaming").join("rivet-fleet").join("register")
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().expect("a parent")).expect("a directory");
    fs::write(path, bytes).expect("written");
}

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn device(id: &str, name: &str) -> Device {
    Device {
        id: id.to_owned(),
        name: name.to_owned(),
        ..Device::default()
    }
}

/// **The whole workstation a fleet leaves behind.**
fn populated(label: &str) -> Home {
    let home = Home::new(label);

    // The authority, as `rivet-provision ca` names its files.
    for name in [
        "root.crt",
        "root.key",
        "browser.crt",
        "browser.key",
        "browser.pfx",
        "suffix",
        "server-arbor.local.crt",
        "server-arbor.local.key",
    ] {
        write(&home.ca().join(name), format!("the authority's {name}").as_bytes());
    }

    // The signing seeds and their public halves.
    for name in ["rivet-system.seed", "rivet-system.pub", "rivet-app.seed", "rivet-app.pub"] {
        write(&home.keys().join(name), format!("key material {name}").as_bytes());
    }

    // The operator identity, and one machine's.
    for dir in ["operator", "arbor"] {
        for name in ["device.seed", "device.crt", "peers"] {
            write(
                &home.identity().join(dir).join(name),
                format!("{dir} {name}").as_bytes(),
            );
        }
    }

    // Provisioning: a composed document, its staging bundle, an answers notebook.
    write(&home.provision().join("arbor.provisioning.toml"), b"hostname = \"arbor\"\n");
    write(&home.provision().join("arbor.staged.txt"), b"a staging bundle\n");
    write(&home.provision().join("vega.toml"), b"# answers so far\n");

    // The operator register, two devices, saved by the register's own store.
    let mut register = Register::empty(home.register());
    register.fleet.name = Some("home".to_owned());
    register.enroll(device("0011223344556677", "arbor")).expect("fresh");
    register.enroll(device("8899aabbccddeeff", "vega")).expect("fresh");
    rivetos_register::store::save(&register).expect("the register saves");

    // The fleet repository, as `cargo rivet fleet init` starts one, then the
    // records `cargo rivet provision` and `rivet-fleet` commit into it.
    let fleet = home.fleet();
    fs::create_dir_all(&fleet).expect("a fleet directory");
    git(&fleet, &["init", "--quiet", "--initial-branch=main"]);
    git(&fleet, &["config", "user.name", "rivet-reset test"]);
    git(&fleet, &["config", "user.email", "test@rivet.invalid"]);
    git(&fleet, &["config", "core.autocrlf", "false"]);
    write(&fleet.join(".gitignore"), b"/objects/\n");
    write(&fleet.join("manifest.toml"), b"# the published artifacts\n[[artifact]]\n");
    write(
        &fleet.join("objects").join("ab".repeat(32)),
        b"the bytes of a signed artifact",
    );
    git(&fleet, &["add", ".gitignore", "manifest.toml"]);
    git(&fleet, &["commit", "--quiet", "-m", "Initialise the fleet repository"]);
    for host in ["arbor", "vega"] {
        let outcome = rivetos_fleet_repo::record_document(
            &fleet,
            host,
            &format!("hostname = \"{host}\"\n"),
        );
        assert!(
            matches!(outcome, rivetos_fleet_repo::Outcome::Committed(_)),
            "{outcome}"
        );
    }
    let outcome = rivetos_fleet_repo::record_register(&fleet, &register);
    assert!(
        matches!(outcome, rivetos_fleet_repo::Outcome::Committed(_)),
        "{outcome}"
    );

    // The Armada, holding one Fleet.
    let mut estate = Armada::empty(home.armada());
    estate.fleets.insert(
        "home".to_owned(),
        armada::Fleet {
            name: "home".to_owned(),
            server: armada::Server {
                host: "arbor.local:7777".to_owned(),
                pin: "ab".repeat(32),
                operator_key: None,
            },
        },
    );
    armada::store::save(&estate).expect("the Armada saves");

    home
}

/// Run the built `rivet-reset` against `home`.
fn reset(home: &Home, extra: &[&str]) -> (Output, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_rivet-reset"))
        .arg("--keep-authority")
        .arg("--register")
        .arg(home.register())
        .args(extra)
        .env("HOME", &home.0)
        .env("USERPROFILE", &home.0)
        .output()
        .expect("rivet-reset runs");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out, text)
}

/// Every file under `dir`, relative path to bytes, never descending into
/// `.git` — git may refresh its own index on a `status`, and that is not a
/// change to anything this program was asked about.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == ".git") {
                    continue;
                }
                stack.push(path);
            } else {
                let relative = path.strip_prefix(dir).expect("under dir").to_path_buf();
                out.insert(relative, fs::read(&path).expect("readable"));
            }
        }
    }
    out
}

/// What must survive, byte for byte.
fn kept(home: &Home) -> BTreeMap<&'static str, BTreeMap<PathBuf, Vec<u8>>> {
    let mut out = BTreeMap::new();
    out.insert("ca", snapshot(&home.ca()));
    out.insert("keys", snapshot(&home.keys()));
    out.insert("identity", snapshot(&home.identity()));
    out.insert("objects", snapshot(&home.fleet().join("objects")));
    let mut single = BTreeMap::new();
    single.insert(
        PathBuf::from("manifest.toml"),
        fs::read(home.fleet().join("manifest.toml")).expect("the manifest"),
    );
    single.insert(
        PathBuf::from("armada.toml"),
        fs::read(armada::store::settings_path(&home.armada())).expect("the Armada settings"),
    );
    out.insert("files", single);
    out
}

/// **Not a test: a scratch workstation left on disk for a manual run.**
///
/// `cargo test -p rivetos-reset --test keep_authority -- --ignored --nocapture`
/// prints the scratch `HOME` it built and the command lines to run against it.
/// It is the same tree every test here builds, and it is left behind on
/// purpose; delete it when done.
#[test]
#[ignore = "builds a scratch workstation for a manual run and leaves it on disk"]
fn a_scratch_workstation_for_a_manual_run() {
    let home = populated("manual");
    let register = home.register();
    let path = home.0.clone();
    std::mem::forget(home);
    println!("scratch HOME: {}", path.display());
    println!("run, with HOME and USERPROFILE set to it:");
    println!(
        "  rivet-reset --keep-authority --register {} --dry-run",
        register.display()
    );
    println!(
        "  rivet-reset --keep-authority --register {} --confirm",
        register.display()
    );
}

/// **A dry run removes nothing and reports the plan**: every location with
/// its count, grouped into what goes and what stays, with each kept thing
/// named.
#[test]
fn a_dry_run_removes_nothing_and_reports_the_plan() {
    let home = populated("dry");
    let before = snapshot(&home.0);

    let (out, text) = reset(&home, &["--dry-run"]);
    assert!(out.status.success(), "{text}");
    assert_eq!(snapshot(&home.0), before, "a dry run changed something\n{text}");

    for expected in [
        "WOULD BE REMOVED",
        "THE OPERATOR REGISTER",
        "THE FLEET REPOSITORY'S MACHINE RECORDS",
        "PROVISIONING NOTEBOOKS AND DOCUMENTS",
        "THE ARMADA'S FLEETS",
        "KEPT — named, and not touched",
        "THE CERTIFICATE AUTHORITY",
        "8 file(s), none of them touched",
        "THE WINDOWS CERTIFICATE STORES",
        "THE FLEET REPOSITORY'S MANIFEST",
        "THE FLEET REPOSITORY'S OBJECTS",
        "THE ARMADA'S SETTINGS",
        "rivet-app.seed, rivet-system.seed",
        "THE OPERATOR IDENTITIES",
        "12 file(s) from 4 location(s) would be removed.",
        "--dry-run: nothing was removed.",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in\n{text}");
    }
    // Per location: 3 in the register, 5 in the repository (2 documents and
    // a 3-file projection), 3 notebooks and documents, 1 Fleet.
    let counts: Vec<&str> = text
        .lines()
        .filter(|l| l.trim_end().ends_with("file(s)") && l.contains(" — "))
        .filter_map(|l| l.rsplit(" — ").next())
        .collect();
    assert_eq!(
        counts,
        ["3 file(s)", "5 file(s)", "3 file(s)", "1 file(s)"],
        "{text}"
    );
    let removed_at = text.find("WOULD BE REMOVED").expect("removed heading");
    let kept_at = text.find("KEPT — named").expect("kept heading");
    assert!(removed_at < kept_at, "{text}");
    assert!(
        !text[removed_at..kept_at].contains("root.key"),
        "the authority appears in the removal plan\n{text}"
    );
}

/// **`--confirm` empties the registers, the documents, the notebooks and the
/// Armada; the authority, the keys, the identities, the manifest and the
/// objects survive byte for byte; the fleet repository gains exactly one
/// commit and is left clean.**
#[test]
fn confirm_empties_the_fleet_and_keeps_the_authority_byte_for_byte() {
    let home = populated("confirm");
    let kept_before = kept(&home);
    let commits_before = git(&home.fleet(), &["rev-list", "--count", "HEAD"]);

    let (out, text) = reset(&home, &["--confirm"]);
    assert!(out.status.success(), "{text}");

    assert!(!home.register().exists(), "the operator register survived\n{text}");
    assert!(!home.fleet().join("register").exists(), "{text}");
    assert!(!home.fleet().join("documents").exists(), "{text}");
    assert!(!home.provision().exists(), "the notebooks survived\n{text}");
    let estate = armada::store::load(&home.armada()).expect("the Armada still reads");
    assert!(estate.fleets.is_empty(), "the Armada still lists {:?}", estate.fleets.keys());

    assert_eq!(kept(&home), kept_before, "something kept was changed\n{text}");

    let commits_after = git(&home.fleet(), &["rev-list", "--count", "HEAD"]);
    assert_eq!(
        commits_after.trim().parse::<u32>().unwrap(),
        commits_before.trim().parse::<u32>().unwrap() + 1,
        "{text}"
    );
    let subject = git(&home.fleet(), &["log", "-1", "--format=%s"]);
    assert!(subject.starts_with("rivet-reset --keep-authority removed"), "{subject}");
    assert_eq!(git(&home.fleet(), &["status", "--porcelain"]), "", "the tree is not clean");

    // **And a second run is a report, not an error.**
    let (again, text) = reset(&home, &["--confirm"]);
    assert!(again.status.success(), "{text}");
    assert!(text.contains("Nothing to remove."), "{text}");
    assert_eq!(
        git(&home.fleet(), &["rev-list", "--count", "HEAD"]),
        commits_after,
        "a second run committed again"
    );
}

/// **A dirty fleet repository is refused by name before ANYTHING is removed**
/// — not only the repository's own records, but the register, the notebooks
/// and the Armada too.
#[test]
fn a_dirty_fleet_repository_is_refused_before_anything_is_removed() {
    for dirty in ["modified", "untracked"] {
        let home = populated(dirty);
        if dirty == "modified" {
            write(&home.fleet().join("manifest.toml"), b"# half-published\n");
        } else {
            write(
                &home.fleet().join("documents").join("new.provisioning.toml"),
                b"hostname = \"new\"\n",
            );
        }
        let before = snapshot(&home.0);

        let (out, text) = reset(&home, &["--confirm"]);
        assert!(!out.status.success(), "a dirty repository was accepted\n{text}");
        assert!(text.contains("REFUSED. Nothing was removed."), "{text}");
        assert!(text.contains("uncommitted changes"), "{text}");
        assert!(
            text.contains(&home.fleet().display().to_string()),
            "the refusal does not name the repository\n{text}"
        );
        assert_eq!(snapshot(&home.0), before, "{dirty}: something was removed\n{text}");
    }
}

/// A directory link at `link` pointing at `target`: a junction on Windows,
/// which `mklink /J` makes without elevation, and a symlink elsewhere.
fn directory_link(link: &Path, target: &Path) {
    #[cfg(windows)]
    {
        let status = Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(link)
            .arg(target)
            .output()
            .expect("mklink runs");
        assert!(status.status.success(), "mklink /J failed: {status:?}");
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).expect("a directory symlink");
}

/// **An alias of the authority passed as `--provision` removes none of it**
/// — the external audit's F02, as a junction on Windows and a symlink
/// elsewhere. Two spellings: the removal root itself a link to `~/.rivet/ca`,
/// and a root reached through a linked PARENT, which is not itself a link and
/// is only caught by comparing resolved paths with the kept roots.
#[test]
fn an_alias_of_the_authority_given_as_provision_removes_none_of_it() {
    for spelling in ["the root is the link", "a parent is the link"] {
        let home = populated(if spelling.starts_with("the root") { "alias-root" } else { "alias-parent" });
        let aliases = home.0.join("aliases");
        fs::create_dir_all(&aliases).expect("a directory for the alias");
        let provision = if spelling.starts_with("the root") {
            let link = aliases.join("ca-alias");
            directory_link(&link, &home.ca());
            link
        } else {
            let link = aliases.join("rivet-alias");
            directory_link(&link, &home.rivet());
            link.join("ca")
        };
        let kept_before = kept(&home);
        let provision = provision.to_str().expect("a UTF-8 scratch path").to_owned();

        let (out, text) = reset(&home, &["--provision", &provision, "--confirm"]);
        assert!(!out.status.success(), "{spelling}: the alias was accepted\n{text}");
        assert!(text.contains("Nothing was removed."), "{spelling}: {text}");
        assert_eq!(
            kept(&home),
            kept_before,
            "{spelling}: something kept was removed\n{text}"
        );
        assert!(home.register().exists(), "{spelling}: the run was not refused whole\n{text}");
    }
}
