#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! Every machine here is a fake [`Host`]: a verdict is a fact about what a
//! host answers, and each test states the answers of one shape of machine.
//! Credentials are real files under a scratch directory, because the survey
//! they go through is `setup --check`'s and is not this module's to fake.

use super::*;
use std::collections::{HashMap, HashSet};

/// A machine described by what it answers.
struct Fake {
    os: &'static str,
    env: HashMap<&'static str, String>,
    on_path: HashSet<&'static str>,
    /// `program arg arg …` → its answer. Anything unlisted fails to start.
    runs: HashMap<String, Result<String, String>>,
    files: HashMap<PathBuf, String>,
    cwd: PathBuf,
    provision: Option<PathBuf>,
    seeds_agree: bool,
}

const PROBE: &str = "docker run --rm --platform linux/amd64 -v rivet-br:/br:ro rivet-card-build:9 sh -c";

impl Fake {
    /// A Linux x86-64 workstation with everything, standing in a checkout.
    fn complete(checkout: &Path) -> Self {
        let mut files = HashMap::new();
        for f in ["cmd/cargo-rivet/Cargo.toml", "rivet-core/Cargo.toml", FIRMWARE[0], FIRMWARE[1]] {
            files.insert(checkout.join(f), String::new());
        }
        files.insert(checkout.join("tools/container/image-version"), "9\n".to_owned());
        files.insert(
            checkout.join(".cargo/config.toml"),
            format!("[target.{TARGET}]\nlinker = \"rust-lld\"\n"),
        );
        let mut runs = HashMap::new();
        let mut answer = |cmd: &str, out: &str| {
            runs.insert(cmd.to_owned(), Ok(out.to_owned()));
        };
        answer("/bin/rivet-provision --version", "rivet-provision 0.1.118 (v0.1.95-545-gabc)");
        answer("rustup show active-toolchain", "1.97.1-x86_64-unknown-linux-gnu (overridden)");
        answer("rustup target list --installed", &format!("x86_64-unknown-linux-gnu\n{TARGET}"));
        answer("docker version --format {{.Server.Arch}}", "amd64");
        answer("docker image inspect --format {{.Id}} rivet-card-build:9", "sha256:1");
        answer("docker volume inspect rivet-br", "[]");
        answer(PROBE, "");
        // The scratch credentials live under the temporary directory, so that
        // is this machine's home — as `~/.rivet` is on a real one.
        let env = HashMap::from([("HOME", std::env::temp_dir().display().to_string())]);
        Fake {
            os: "linux",
            env,
            on_path: ["rustup", "docker", "git", "uv"].into_iter().collect(),
            runs,
            files,
            cwd: checkout.to_path_buf(),
            provision: Some(PathBuf::from("/bin/rivet-provision")),
            seeds_agree: true,
        }
    }
}

impl Host for Fake {
    fn os(&self) -> &str {
        self.os
    }
    fn arch(&self) -> &str {
        "x86_64"
    }
    fn env(&self, name: &str) -> Option<String> {
        self.env.get(name).cloned()
    }
    fn which(&self, program: &str) -> Option<PathBuf> {
        self.on_path.contains(program).then(|| PathBuf::from("/bin").join(program))
    }
    fn run(&self, program: &str, args: &[&str], _cwd: Option<&Path>) -> Result<String, String> {
        // The Buildroot probe's shell script is matched by its prefix: the
        // script is this module's own text and restating it here would test
        // nothing but that it was copied correctly.
        let full = std::iter::once(program).chain(args.iter().copied()).collect::<Vec<_>>().join(" ");
        let key = self.runs.keys().find(|k| full == **k || full.starts_with(&format!("{k} "))).cloned();
        key.map_or_else(|| Err(format!("{program} would not start")), |k| self.runs[&k].clone())
    }
    fn is_file(&self, path: &Path) -> bool {
        self.files.contains_key(path)
    }
    fn read(&self, path: &Path) -> Option<String> {
        self.files.get(path).cloned()
    }
    fn cwd(&self) -> Option<PathBuf> {
        Some(self.cwd.clone())
    }
    fn provision_sibling(&self) -> Option<PathBuf> {
        self.provision.clone()
    }
    fn this_binary(&self) -> Option<PathBuf> {
        Some(PathBuf::from("/bin/cargo-rivet"))
    }
    fn cargo_rivet_on_path(&self) -> Option<PathBuf> {
        Some(PathBuf::from("/bin/cargo-rivet"))
    }
    fn same_file(&self, a: &Path, b: &Path) -> bool {
        a == b
    }
    fn seed_agrees(&self, _seed: &Path, _published: &Path) -> Result<bool, String> {
        Ok(self.seeds_agree)
    }
}

const BUILD: &str = "0.1.118 (v0.1.95-545-gabc)";

/// A `~/.rivet` holding every credential `setup` mints, as real files.
fn credentials_root(name: &str) -> PathBuf {
    let at = std::env::temp_dir().join(format!("rivet-doctor-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&at);
    std::fs::create_dir_all(at.join("keys")).unwrap();
    for seed in ["rivet-system.seed", "rivet-app.seed"] {
        std::fs::write(at.join("keys").join(seed), [7u8; 32]).unwrap();
    }
    for who in ["operator", "board"] {
        let dir = at.join("identity").join(who);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("device.seed"), [9u8; 32]).unwrap();
        std::fs::write(dir.join("device.crt"), b"x").unwrap();
        std::fs::write(dir.join("peers"), b"x").unwrap();
    }
    std::fs::create_dir_all(at.join("ca")).unwrap();
    std::fs::write(at.join("ca").join("root.crt"), b"x").unwrap();
    at
}

fn verdicts(host: &Fake, goals: &[Goal], at: &Path) -> Vec<(Goal, Vec<Finding>)> {
    evaluate(host, goals, Some(at), &Overrides::default(), BUILD)
}

fn failures(report: &[(Goal, Vec<Finding>)], goal: Goal) -> Vec<String> {
    report
        .iter()
        .filter(|(g, _)| *g == goal)
        .flat_map(|(_, f)| f.iter())
        .filter(|f| f.status == Status::Fail)
        .map(|f| f.what.clone())
        .collect()
}

fn first_failure(report: &[(Goal, Vec<Finding>)]) -> Finding {
    report.iter().flat_map(|(_, f)| f.iter()).find(|f| f.status == Status::Fail).expect("a failure").clone()
}

#[test]
fn a_complete_linux_workstation_is_ready_for_every_goal() {
    let at = credentials_root("complete");
    let host = Fake::complete(Path::new("/src/rivet"));
    let report = verdicts(&host, &Goal::ALL, &at);
    for goal in Goal::ALL {
        assert!(failures(&report, goal).is_empty(), "{goal:?}: {:?}", failures(&report, goal));
    }
    let (text, failed) = render(&host, &report, BUILD, false);
    assert_eq!(failed, 0);
    assert!(text.contains("Nothing to fix."), "{text}");
}

/// The Mac this verb was written on: an arm64 daemon, so the images run
/// emulated, and a card cannot be written to a device. Neither stops a goal.
#[test]
fn a_mac_is_told_about_emulation_and_card_writing_without_failing() {
    let at = credentials_root("mac");
    let mut host = Fake::complete(Path::new("/src/rivet"));
    host.os = "macos";
    host.runs.insert("docker version --format {{.Server.Arch}}".to_owned(), Ok("arm64".to_owned()));
    let report = verdicts(&host, &Goal::ALL, &at);
    let (text, failed) = render(&host, &report, BUILD, false);
    assert_eq!(failed, 0, "{text}");
    assert!(text.contains("✓ can write cards (sudo asks for your password"), "{text}");
    assert!(text.contains("x86-64 images run emulated on this arm64 host"), "{text}");
    assert!(text.contains("macOS"), "the header names the platform as a person spells it: {text}");
}

/// The readability contract: the reasoning is there, and only on request. FreeBSD,
/// because it is a platform with a card-writing caveat and therefore a reason to
/// print.
#[test]
fn the_reasoning_is_printed_only_with_verbose() {
    let at = credentials_root("verbose");
    let mut host = Fake::complete(Path::new("/src/rivet"));
    host.os = "freebsd";
    host.runs.insert("docker version --format {{.Server.Arch}}".to_owned(), Ok("arm64".to_owned()));
    let report = verdicts(&host, &Goal::ALL, &at);
    let (quiet, _) = render(&host, &report, BUILD, false);
    let (loud, _) = render(&host, &report, BUILD, true);
    for reason in ["Rosetta", "Removable-disk enumeration"] {
        assert!(!quiet.contains(reason), "{reason:?} leaked into the short report:\n{quiet}");
        assert!(loud.contains(reason), "{reason:?} is missing from --verbose:\n{loud}");
    }
    assert!(quiet.contains("--verbose"), "the short report says how to get the long one");
}

/// Every line of a real report fits a terminal, which the absolute paths of
/// the first version of this report did not.
#[test]
fn no_line_is_wider_than_a_terminal() {
    let at = credentials_root("w");
    let mut host = Fake::complete(Path::new("/src/rivet"));
    host.seeds_agree = false;
    host.on_path.remove("rustup");
    let report = verdicts(&host, &Goal::ALL, &at);
    for verbose in [false, true] {
        let (text, _) = render(&host, &report, BUILD, verbose);
        for line in text.lines() {
            assert!(line.chars().count() <= 100, "{} chars: {line}", line.chars().count());
        }
    }
}

/// Someone who only wants to reach a board has no Docker. Only `image` may
/// fail for that, once, and `--for run` must then exit clean.
#[test]
fn no_container_runtime_fails_image_and_nothing_else() {
    let at = credentials_root("nodocker");
    let mut host = Fake::complete(Path::new("/src/rivet"));
    host.on_path.remove("docker");
    let report = verdicts(&host, &Goal::ALL, &at);
    assert!(failures(&report, Goal::Run).is_empty());
    assert!(failures(&report, Goal::App).is_empty());
    assert_eq!(failures(&report, Goal::Image), vec!["docker is not installed".to_owned()]);
    assert!(
        !report[2].1.iter().any(|f| f.what.contains("Buildroot") || f.what.contains("build image")),
        "nothing that depends on the runtime is reported as a second problem"
    );

    let (_, failed) = render(&host, &verdicts(&host, &[Goal::Run], &at), BUILD, false);
    assert_eq!(failed, 0);
}

#[test]
fn a_stopped_daemon_is_named_as_the_cause_once() {
    let at = credentials_root("daemon");
    let mut host = Fake::complete(Path::new("/src/rivet"));
    host.runs.insert(
        "docker version --format {{.Server.Arch}}".to_owned(),
        Err("Cannot connect to the Docker daemon".to_owned()),
    );
    let report = verdicts(&host, &[Goal::Image], &at);
    assert_eq!(failures(&report, Goal::Image), vec!["docker is not running".to_owned()]);
}

/// Outside a checkout the toolchain is usually `stable`, which has its own
/// target list — the case that failed on the Mac this was written on.
#[test]
fn a_toolchain_without_the_musl_target_fails_app_and_names_that_toolchain() {
    let at = credentials_root("target");
    let mut host = Fake::complete(Path::new("/src/rivet"));
    host.runs.insert("rustup show active-toolchain".to_owned(), Ok("stable-aarch64-apple-darwin (default)".to_owned()));
    host.runs.insert("rustup target list --installed".to_owned(), Ok("aarch64-apple-darwin".to_owned()));
    let finding = first_failure(&verdicts(&host, &[Goal::App], &at));
    assert!(finding.what.contains("stable-aarch64-apple-darwin"), "{finding:?}");
    assert_eq!(
        finding.fixes,
        vec!["rustup target add aarch64-unknown-linux-musl --toolchain stable-aarch64-apple-darwin".to_owned()]
    );
}

/// A crate outside a checkout inherits no linker setting. It is a caveat,
/// because `doctor` may be run from somewhere that is not the crate at all.
#[test]
fn a_crate_outside_a_checkout_is_warned_about_the_linker() {
    let at = credentials_root("linker");
    let mut host = Fake::complete(Path::new("/src/rivet"));
    host.cwd = PathBuf::from("/elsewhere/my-app");
    let report = verdicts(&host, &[Goal::App], &at);
    let linker = report[0].1.iter().find(|f| f.what.contains("rust-lld")).expect("the linker finding");
    assert_eq!(linker.status, Status::Warn, "{linker:?}");
    assert!(linker.fixes[0].contains("linker = \"rust-lld\""), "{linker:?}");

    host.env.insert("CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER", "rust-lld".to_owned());
    let report = verdicts(&host, &[Goal::App], &at);
    assert!(report[0].1.iter().any(|f| f.status == Status::Ok && f.what.contains("$CARGO_TARGET_")));
}

#[test]
fn image_outside_a_checkout_fails_on_that_alone() {
    let at = credentials_root("nocheckout");
    let mut host = Fake::complete(Path::new("/src/rivet"));
    host.cwd = PathBuf::from("/elsewhere");
    let report = verdicts(&host, &[Goal::Image], &at);
    assert_eq!(failures(&report, Goal::Image), vec!["not in a Rivet checkout".to_owned()]);
    assert_eq!(report[0].1.len(), 1, "nothing else is judged without a checkout");
}

#[test]
fn missing_firmware_names_the_files_and_the_fetch() {
    let at = credentials_root("firmware");
    let mut host = Fake::complete(Path::new("/src/rivet"));
    host.files.remove(&PathBuf::from("/src/rivet").join(FIRMWARE[1]));
    let finding = first_failure(&verdicts(&host, &[Goal::Image], &at));
    assert!(finding.note.as_deref().unwrap_or("").contains("fixup4.dat"), "{finding:?}");
    assert_eq!(finding.fixes, vec!["tools/fetch-firmware.sh".to_owned()]);
}

/// The quiet failure this check exists for: a card trusts `keys/*.pub` and
/// would refuse everything its builder signs. Both ways out are offered.
#[test]
fn committed_keys_that_are_not_yours_fail_image() {
    let at = credentials_root("keys");
    let mut host = Fake::complete(Path::new("/src/rivet"));
    host.seeds_agree = false;
    let report = verdicts(&host, &[Goal::Image], &at);
    let failed = failures(&report, Goal::Image);
    assert_eq!(failed.len(), 2, "{failed:?}");
    assert!(failed.iter().all(|w| w.starts_with("keys/")), "{failed:?}");
    let finding = first_failure(&report);
    assert_eq!(finding.fixes.len(), 2, "{finding:?}");
    assert!(finding.fixes[0].contains("key restore"), "{finding:?}");
}

#[test]
fn an_absent_or_incomplete_buildroot_volume_fails_image() {
    let at = credentials_root("buildroot");
    let mut host = Fake::complete(Path::new("/src/rivet"));
    host.runs.remove("docker volume inspect rivet-br");
    let report = verdicts(&host, &[Goal::Image], &at);
    assert_eq!(failures(&report, Goal::Image), vec!["no Buildroot volume (rivet-br)".to_owned()]);

    let mut host = Fake::complete(Path::new("/src/rivet"));
    host.runs.insert(PROBE.to_owned(), Ok("images/Image\nimages/bcm2711-rpi-4-b.dtb".to_owned()));
    let finding = first_failure(&verdicts(&host, &[Goal::Image], &at));
    assert!(
        finding.note.as_deref().unwrap_or("").contains("images/Image, images/bcm2711-rpi-4-b.dtb"),
        "{finding:?}"
    );
}

#[test]
fn a_mismatched_or_missing_rivet_provision_fails_run() {
    let at = credentials_root("provision");
    let mut host = Fake::complete(Path::new("/src/rivet"));
    host.runs.insert(
        "/bin/rivet-provision --version".to_owned(),
        Ok("rivet-provision 0.1.100 (v0.1.95-1-gold)".to_owned()),
    );
    assert_eq!(
        failures(&verdicts(&host, &[Goal::Run], &at), Goal::Run),
        vec!["rivet-provision is a different build".to_owned()]
    );

    host.provision = None;
    assert_eq!(
        failures(&verdicts(&host, &[Goal::Run], &at), Goal::Run),
        vec!["rivet-provision is not installed".to_owned()]
    );
}

/// `-dirty` on one of the pair is routine for a developer with an edit in
/// flight, and `provision` itself does not refuse it — so neither may this.
#[test]
fn a_pair_differing_only_by_dirty_is_the_same_build() {
    let at = credentials_root("dirty");
    let mut host = Fake::complete(Path::new("/src/rivet"));
    host.runs.insert(
        "/bin/rivet-provision --version".to_owned(),
        Ok("rivet-provision 0.1.118 (v0.1.95-545-gabc-dirty)".to_owned()),
    );
    assert!(failures(&verdicts(&host, &[Goal::Run], &at), Goal::Run).is_empty());
}

/// Credentials that are all present are one line, not five.
#[test]
fn present_credentials_collapse_to_one_line() {
    let at = credentials_root("collapse");
    let host = Fake::complete(Path::new("/src/rivet"));
    let report = verdicts(&host, &[Goal::Run], &at);
    let lines = report[0].1.iter().filter(|f| f.what.contains("seed") || f.what.contains("identity")).count();
    assert_eq!(lines, 1, "{:#?}", report[0].1);
}

#[test]
fn missing_credentials_fail_run_and_name_the_fix() {
    let at = std::env::temp_dir().join(format!("rivet-doctor-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&at);
    std::fs::create_dir_all(&at).unwrap();
    let host = Fake::complete(Path::new("/src/rivet"));
    let report = verdicts(&host, &[Goal::Run], &at);
    let fails: Vec<&Finding> = report[0].1.iter().filter(|f| f.status == Status::Fail).collect();
    assert!(!fails.is_empty());
    assert!(fails.iter().all(|f| !f.fixes.is_empty()), "{fails:?}");
}

#[test]
fn for_narrows_the_report_and_refuses_an_unknown_goal() {
    let words = |w: &[&str]| w.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    assert_eq!(parse(&words(&[])).unwrap().goals, Goal::ALL.to_vec());
    assert_eq!(parse(&words(&["--for", "app", "--for", "app"])).unwrap().goals, vec![Goal::App]);
    assert!(parse(&words(&["-v"])).unwrap().verbose);
    assert!(parse(&words(&["--for", "sdk"])).unwrap_err().contains("run, app and image"));
    assert!(parse(&words(&["--for"])).is_err());
}
