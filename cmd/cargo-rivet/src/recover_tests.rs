// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

//! Every fleet repository, register and backup folder here is HAND-COMPOSED in
//! a scratch directory: the register through `rivetos_register::render`, the
//! manifest in the shape `fleet::read_manifest` reads, and objects whose
//! digests are computed here. No real repository is read.

use std::cell::RefCell;

use super::*;

const HOST: &str = "recover-host";
const APP: &str = "restore-probe";
const OTHER: &str = "other-probe";
/// A fictional pin. Nothing here connects to anything.
const PIN: &str = "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";
const OTHER_PIN: &str = "b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2";

fn scratch(label: &str) -> PathBuf {
    crate::scratch::dir(&format!(
        "recover-{label}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ))
}

fn blake3_hex(bytes: &[u8]) -> String {
    rivet_core::package::key_material::to_hex(&rivet_crypto::Blake3::new().of(bytes))
}

/// A fleet repository holding `HOST`'s document, which installs `APP` and
/// `OTHER`, one published artifact of each, and a register recording `HOST`
/// with `PIN`; and a backup folder with data for both.
struct Fixture {
    root: PathBuf,
    from: PathBuf,
}

fn document(host: &str) -> String {
    format!(
        "version = 1\nhostname = \"{host}\"\n\n[services.{APP}]\nenabled = true\n\n[services.{OTHER}]\nenabled = true\n"
    )
}

impl Fixture {
    fn new(label: &str) -> Self {
        let base = scratch(label);
        let root = base.join("fleet");
        let documents = root.join(rivetos_fleet_repo::DOCUMENTS_DIR);
        std::fs::create_dir_all(&documents).unwrap();
        std::fs::write(documents.join(format!("{HOST}{}", rivetos_fleet_repo::DOCUMENT_SUFFIX)), document(HOST))
            .unwrap();
        std::fs::create_dir_all(root.join("objects")).unwrap();
        let mut manifest = String::new();
        for (product, bytes) in [(APP, b"the restore probe's artifact".as_slice()), (OTHER, b"another artifact".as_slice())] {
            let digest = blake3_hex(bytes);
            std::fs::write(root.join("objects").join(&digest), bytes).unwrap();
            manifest.push_str(&format!(
                "[[artifact]]\ndigest = \"{digest}\"\nproduct = \"{product}\"\nversion = \"0.1.0\"\narch = \"aarch64\"\nkind = \"application\"\nsize = {}\nadded = \"2000-01-01T00:00:00Z\"\n\n",
                bytes.len()
            ));
        }
        std::fs::write(root.join("manifest.toml"), manifest).unwrap();
        let fixture = Self { root, from: base.join("backups") };
        fixture.register(Some(PIN));
        for app in [APP, OTHER] {
            std::fs::create_dir_all(fixture.from.join(app)).unwrap();
            std::fs::write(fixture.from.join(app).join(format!("{app}.sqlite")), format!("{app} data")).unwrap();
        }
        fixture
    }

    /// The register's projection, rendered by the register's own renderer.
    fn register(&self, pin: Option<&str>) {
        let projected = self.root.join(rivetos_fleet_repo::REGISTER_DIR);
        std::fs::remove_dir_all(&projected).ok();
        let mut register = rivetos_register::Register::empty(&projected);
        register
            .enroll(rivetos_register::Device {
                id: "0011223344556677".to_owned(),
                name: HOST.to_owned(),
                pin: pin.map(str::to_owned),
                ..rivetos_register::Device::default()
            })
            .unwrap();
        for (path, text) in rivetos_register::render(&register).unwrap() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }

    fn args(&self) -> RecoverArgs {
        RecoverArgs { host: HOST.to_owned(), from: self.from.clone(), ..RecoverArgs::default() }
    }

    fn plan(&self, args: &RecoverArgs) -> Result<Plan, String> {
        plan(args, &self.root)
    }
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_owned).collect()
}

// ---------------------------------------------------------------------------
// The plan: everything it would use, and its refusals
// ---------------------------------------------------------------------------

#[test]
fn a_complete_plan_names_every_step_and_the_card_is_never_confirmed_for_provision() {
    let f = Fixture::new("complete");
    let args = f.args();
    let plan = f.plan(&args).expect("a plan");
    assert_eq!(plan.pin, PIN);
    assert!(plan.recorded);
    let products: Vec<&str> = plan.applications.iter().map(|a| a.product.as_str()).collect();
    assert_eq!(products, [OTHER, APP]);
    for app in &plan.applications {
        let object = f.root.join("objects").join(&app.digest);
        assert_eq!(blake3_hex(&std::fs::read(object).unwrap()), app.digest);
    }
    let data: Vec<&str> = plan.data.iter().map(|d| d.application.as_str()).collect();
    assert_eq!(data, [OTHER, APP]);

    let text = render(&plan, &args);
    for needle in ["1  the card", "2  the secrets", "3  the applications", "4  the data", "--existing recover-host", "Nothing was written"] {
        assert!(text.contains(needle), "{needle:?} missing from:\n{text}");
    }
    assert!(!card_words(&plan, &args).iter().any(|w| w == "confirm"), "provision must ask its own question");
}

#[test]
fn a_host_with_no_recorded_document_is_refused_naming_the_hosts_that_are() {
    let f = Fixture::new("no-document");
    let mut args = f.args();
    args.host = "nobody".to_owned();
    let err = f.plan(&args).expect_err("refused");
    assert!(err.contains("NOT IN THE FLEET REPOSITORY"), "{err}");
    assert!(err.contains(HOST), "it names what is recorded: {err}");
    assert!(err.contains("NOTHING WAS WRITTEN"), "{err}");
}

#[test]
fn a_document_filed_under_the_host_that_names_another_host_is_refused() {
    let f = Fixture::new("other-hostname");
    std::fs::write(
        f.root.join(rivetos_fleet_repo::DOCUMENTS_DIR).join(format!("{HOST}{}", rivetos_fleet_repo::DOCUMENT_SUFFIX)),
        document("somebody-else"),
    )
    .unwrap();
    let err = f.plan(&f.args()).expect_err("refused");
    assert!(err.contains("somebody-else"), "{err}");
}

#[test]
fn a_host_not_in_the_register_with_no_pin_given_is_refused() {
    let f = Fixture::new("no-register");
    std::fs::remove_dir_all(f.root.join(rivetos_fleet_repo::REGISTER_DIR)).unwrap();
    let err = f.plan(&f.args()).expect_err("refused");
    assert!(err.contains("NOT IN THE REGISTER"), "{err}");

    // Given a pin, the same host plans: the document is what says it exists.
    let mut args = f.args();
    args.endpoint.pin = Some(PIN.to_owned());
    let plan = f.plan(&args).expect("a plan with the pin given");
    assert!(!plan.recorded);
    assert_eq!(plan.pin, PIN);
}

#[test]
fn a_pin_the_register_disagrees_with_is_refused() {
    let f = Fixture::new("pin-mismatch");
    let mut args = f.args();
    args.endpoint.pin = Some(OTHER_PIN.to_owned());
    let err = f.plan(&args).expect_err("refused");
    assert!(err.contains(PIN) && err.contains(OTHER_PIN), "{err}");
}

#[test]
fn an_artifact_the_document_installs_and_the_repository_does_not_publish_is_refused() {
    let f = Fixture::new("unpublished");
    let manifest = std::fs::read_to_string(f.root.join("manifest.toml")).unwrap();
    let kept: String = manifest.split("\n\n").filter(|t| !t.contains(OTHER)).collect::<Vec<_>>().join("\n\n");
    std::fs::write(f.root.join("manifest.toml"), kept).unwrap();
    let err = f.plan(&f.args()).expect_err("refused");
    assert!(err.contains(OTHER) && err.contains("NOTHING WAS WRITTEN"), "{err}");
}

#[test]
fn an_artifact_whose_object_is_absent_is_refused() {
    let f = Fixture::new("object-absent");
    let digest = blake3_hex(b"another artifact");
    std::fs::remove_file(f.root.join("objects").join(&digest)).unwrap();
    let err = f.plan(&f.args()).expect_err("refused");
    assert!(err.contains(&digest), "{err}");
}

#[test]
fn an_artifact_whose_object_does_not_hash_to_its_digest_is_refused() {
    let f = Fixture::new("object-damaged");
    let digest = blake3_hex(b"another artifact");
    std::fs::write(f.root.join("objects").join(&digest), b"damaged").unwrap();
    let err = f.plan(&f.args()).expect_err("refused");
    assert!(err.contains("damaged") && err.contains(&digest), "{err}");
}

#[test]
fn data_for_an_application_the_document_does_not_install_is_refused() {
    let f = Fixture::new("stray-data");
    std::fs::create_dir_all(f.from.join("tempmon")).unwrap();
    std::fs::write(f.from.join("tempmon").join("tempmon.sqlite"), b"x").unwrap();
    let err = f.plan(&f.args()).expect_err("refused");
    assert!(err.contains("tempmon") && err.contains("does not install"), "{err}");
}

#[test]
fn an_application_data_directory_with_no_file_is_refused() {
    let f = Fixture::new("empty-data");
    std::fs::remove_file(f.from.join(APP).join(format!("{APP}.sqlite"))).unwrap();
    let err = f.plan(&f.args()).expect_err("refused");
    assert!(err.contains("holds no backup file"), "{err}");
}

#[test]
fn a_secrets_directory_with_no_sealed_backup_is_refused() {
    let f = Fixture::new("no-secrets");
    let mut args = f.args();
    let empty = scratch("no-secrets-dir");
    args.secrets = Some(empty);
    let err = f.plan(&args).expect_err("refused");
    assert!(err.contains("--secrets") && err.contains(crate::machine_secrets::BACKUP_FILE), "{err}");
}

#[test]
fn a_replace_naming_an_application_with_no_data_is_refused() {
    let f = Fixture::new("replace-unknown");
    let mut args = f.args();
    args.replace = vec!["tempmon".to_owned()];
    let err = f.plan(&args).expect_err("refused");
    assert!(err.contains("--replace tempmon"), "{err}");
}

#[test]
fn rivet_monitors_layout_is_read_and_the_newest_file_is_chosen() {
    let f = Fixture::new("monitor");
    let monitor = f.from.join(HOST);
    for app in [APP, OTHER] {
        std::fs::create_dir_all(monitor.join(app)).unwrap();
        std::fs::write(monitor.join(app).join("older.sqlite"), b"older").unwrap();
    }
    // A coarse clock must not let the two files tie.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(monitor.join(APP).join("newer.sqlite"), b"newer").unwrap();
    std::fs::write(monitor.join(APP).join("newest.sqlite.partial"), b"half").unwrap();
    let plan = f.plan(&f.args()).expect("a plan");
    assert_eq!(plan.data_dir, monitor);
    let app = plan.data.iter().find(|d| d.application == APP).unwrap();
    assert!(app.file.ends_with("newer.sqlite"), "{:?}", app.file);
    assert_eq!(app.passed_over, 1);
    assert!(render(&plan, &f.args()).contains("1 older passed over"));
}

// ---------------------------------------------------------------------------
// The command line
// ---------------------------------------------------------------------------

#[test]
fn the_command_line_takes_a_hostname_first_and_confirm_last() {
    let parsed = parse(&words("box --from d --replace a --replace a --after-boot confirm")).expect("valid");
    assert_eq!(parsed.host, "box");
    assert!(parsed.after_boot && parsed.confirm);
    assert_eq!(parsed.replace, ["a"]);
    assert!(parse(&words("--from d")).expect_err("no host").contains("HOSTNAME"));
    assert!(parse(&words("box")).expect_err("no from").contains("--from is required"));
    assert!(parse(&words("box --from d confirm --after-boot")).expect_err("not last").contains("LAST"));
    assert!(parse(&words("box --from d --after-boot --image x")).is_err());
    assert!(parse(&words("box --from d --passphrase x")).expect_err("no such flag").contains("typed"));
}

// ---------------------------------------------------------------------------
// Steps 2 to 4: the order, and the refusal to write over data
// ---------------------------------------------------------------------------

/// A machine that records every step it is asked for.
#[derive(Default)]
struct FakeMachine {
    calls: RefCell<Vec<String>>,
    holds: Vec<(String, Holds)>,
    refuse_install: bool,
}

impl Machine for FakeMachine {
    fn restore_secrets(&mut self, dir: &Path, replace: bool) -> Result<String, String> {
        self.calls.borrow_mut().push(format!("secrets {} {replace}", dir.display()));
        Ok("secrets restored\n".to_owned())
    }
    fn install(&mut self, application: &Application) -> Result<String, String> {
        self.calls.borrow_mut().push(format!("install {}", application.product));
        if self.refuse_install {
            return Err("the machine refused the install".to_owned());
        }
        Ok("installed\n".to_owned())
    }
    fn converge(&mut self) -> Result<String, String> {
        self.calls.borrow_mut().push("converge".to_owned());
        Ok("{}".to_owned())
    }
    fn holds(&mut self, application: &str) -> Holds {
        self.calls.borrow_mut().push(format!("holds {application}"));
        self.holds
            .iter()
            .find(|(name, _)| name == application)
            .map_or(Holds::Nothing, |(_, holds)| holds.clone())
    }
    fn restore_data(&mut self, application: &str, _file: &Path) -> Result<String, String> {
        self.calls.borrow_mut().push(format!("restore {application}"));
        Ok("restored\n".to_owned())
    }
}

impl FakeMachine {
    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
    fn restores(&self) -> usize {
        self.calls().iter().filter(|c| c.starts_with("restore ")).count()
    }
}

fn planned(label: &str) -> (Fixture, Plan, RecoverArgs) {
    let f = Fixture::new(label);
    let mut args = f.args();
    args.after_boot = true;
    let mut plan = f.plan(&args).expect("a plan");
    // The sealed file is checked by the plan; the fake machine never opens it.
    plan.secrets = Some(PathBuf::from("secrets-dir"));
    (f, plan, args)
}

#[test]
fn the_steps_run_secrets_then_each_install_then_one_converge_then_the_data() {
    let (_f, plan, args) = planned("order");
    let mut machine = FakeMachine::default();
    let report = after_boot(&plan, &args, &mut machine).expect("recovered");
    assert_eq!(
        machine.calls(),
        [
            "secrets secrets-dir false",
            &format!("install {OTHER}"),
            &format!("install {APP}"),
            "converge",
            &format!("holds {OTHER}"),
            &format!("holds {APP}"),
            &format!("restore {OTHER}"),
            &format!("restore {APP}"),
        ]
    );
    assert!(report.contains("every step finished"), "{report}");
}

#[test]
fn a_refused_install_stops_before_the_converge_and_before_any_data() {
    let (_f, plan, args) = planned("install-refused");
    let mut machine = FakeMachine { refuse_install: true, ..FakeMachine::default() };
    let err = after_boot(&plan, &args, &mut machine).expect_err("stopped");
    assert!(err.contains("STOPPED AT STEP 3"), "{err}");
    assert_eq!(machine.calls(), ["secrets secrets-dir false", &format!("install {OTHER}")]);
}

#[test]
fn an_application_holding_other_bytes_refuses_every_restore_without_replace() {
    let (_f, plan, args) = planned("overwrite");
    let mut machine = FakeMachine {
        holds: vec![(APP.to_owned(), Holds::Bytes(b"live data the backup does not hold".to_vec()))],
        ..FakeMachine::default()
    };
    let err = after_boot(&plan, &args, &mut machine).expect_err("refused");
    assert!(err.contains("ASKS FIRST") && err.contains(&format!("--replace {APP}")), "{err}");
    assert!(err.contains("NO APPLICATION WAS SENT ANY DATA"), "{err}");
    // OTHER holds nothing and would have been restored; it was not, because
    // nothing is sent until every application is settled.
    assert_eq!(machine.restores(), 0, "{:?}", machine.calls());
}

#[test]
fn an_application_that_cannot_say_what_it_holds_is_refused_without_replace() {
    let (_f, plan, args) = planned("unknown");
    let mut machine = FakeMachine {
        holds: vec![(APP.to_owned(), Holds::Unknown("connection refused".to_owned()))],
        ..FakeMachine::default()
    };
    let err = after_boot(&plan, &args, &mut machine).expect_err("refused");
    assert!(err.contains("could not say what it holds") && err.contains("connection refused"), "{err}");
    assert_eq!(machine.restores(), 0);
}

#[test]
fn replace_names_the_one_application_whose_data_may_be_replaced() {
    let (_f, plan, mut args) = planned("replace");
    args.replace = vec![APP.to_owned()];
    let mut machine = FakeMachine {
        holds: vec![(APP.to_owned(), Holds::Bytes(b"live".to_vec()))],
        ..FakeMachine::default()
    };
    let report = after_boot(&plan, &args, &mut machine).expect("recovered");
    assert!(report.contains("REPLACING"), "{report}");
    assert_eq!(machine.restores(), 2);
}

#[test]
fn an_application_already_holding_the_backups_bytes_is_not_sent_them() {
    let (f, plan, args) = planned("identical");
    let bytes = std::fs::read(f.from.join(APP).join(format!("{APP}.sqlite"))).unwrap();
    let mut machine = FakeMachine { holds: vec![(APP.to_owned(), Holds::Bytes(bytes))], ..FakeMachine::default() };
    let report = after_boot(&plan, &args, &mut machine).expect("recovered");
    assert!(report.contains("already holds these"), "{report}");
    assert!(!machine.calls().contains(&format!("restore {APP}")), "{:?}", machine.calls());
    assert!(machine.calls().contains(&format!("restore {OTHER}")));
}
