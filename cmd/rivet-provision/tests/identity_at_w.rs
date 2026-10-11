//! **`.rulings/` R-189 D2, through the real binary**: the interview mints a
//! machine's identity at `w` and not before, and refuses at `w` an identity
//! another machine's register record carries.
//!
//! Every run is a scratch `RIVET_AT` with answers piped to stdin. `HOME` and
//! `USERPROFILE` name a directory that must never come to exist, so a read
//! through either is a failed assertion rather than a read of this host's
//! real `~/.rivet`.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A scratch directory that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let at = std::env::temp_dir()
            .join(format!("rivet-provision-identity-at-w-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&at);
        std::fs::create_dir_all(&at).expect("a scratch directory");
        Self(at)
    }

    fn at(&self) -> PathBuf {
        self.0.join("at")
    }

    fn sentinel_home(&self) -> PathBuf {
        self.0.join("home-must-never-exist")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A self-signed Ed25519 certificate in `dir/device.crt`, answering its pin
/// as hex. No seed: nothing these runs do opens one.
fn certificate_in(dir: &Path) -> String {
    std::fs::create_dir_all(dir).expect("a directory");
    let pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).expect("a keypair");
    let params = rcgen::CertificateParams::new(vec!["rivet-device".to_owned()]).expect("params");
    let der = params.self_signed(&pair).expect("self-signed").der().to_vec();
    std::fs::write(dir.join(rivetos_tls::CERTIFICATE_FILE), &der).expect("written");
    rivetos_tls::encode_hex(&rivetos_tls::public_key_in_certificate(&der).expect("a public key"))
}

/// A fleet authority under `RIVET_AT`, so the first-provision guard
/// (`ADR-063` §8) finds one and asks nothing. These runs are about identities,
/// not the authority, and none of them signs with it.
fn authority_in(scratch: &Scratch) {
    let ca = scratch.at().join("ca");
    std::fs::create_dir_all(&ca).expect("a directory");
    let pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).expect("a keypair");
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).expect("params");
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let root = params.self_signed(&pair).expect("self-signed");
    std::fs::write(ca.join("root.crt"), root.pem()).expect("written");
}

/// Run the interview with `answers` on stdin, answering (exit success, stdout).
fn interview(scratch: &Scratch, register: Option<&Path>, answers: &[&str]) -> (bool, String) {
    interview_with(scratch, register, &[], answers)
}

/// [`interview`], with command-line arguments.
fn interview_with(
    scratch: &Scratch,
    register: Option<&Path>,
    args: &[&str],
    answers: &[&str],
) -> (bool, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rivet-provision"));
    command
        .args(args)
        .env("RIVET_AT", scratch.at())
        .env("RIVET_FLEET_REPOSITORY", scratch.0.join("fleet"))
        .env("HOME", scratch.sentinel_home())
        .env("USERPROFILE", scratch.sentinel_home())
        .env_remove("RIVET_PROVISION_ORCHESTRATED")
        .env_remove("RIVET_FLEET_REGISTER")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(register) = register {
        command.env("RIVET_FLEET_REGISTER", register);
    }
    let mut child = command.spawn().expect("rivet-provision starts");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        for answer in answers {
            writeln!(stdin, "{answer}").expect("an answer");
        }
    }
    let output = child.wait_with_output().expect("rivet-provision exits");
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!scratch.sentinel_home().exists(), "a run read or wrote through HOME:\n{said}");
    (output.status.success(), said)
}

/// **`ADR-063` §8, through the real binary: with no terminal and no
/// `--new-fleet`, a workstation with no authority refuses and mints nothing**
/// — however it is answered, because a pipe cannot be asked.
#[test]
fn a_piped_run_with_no_authority_refuses_and_mints_nothing() {
    let scratch = Scratch::new("guard-refuses");
    certificate_in(&scratch.at().join("identity").join("operator"));
    let (ok, said) = interview(&scratch, None, &["1", "y", "rehearsal-fleet"]);
    assert!(!ok, "a piped run with no authority must fail:\n{said}");
    assert!(said.contains("NOTHING WAS MINTED"), "{said}");
    assert!(said.contains("--new-fleet"), "the refusal must name the way to say new:\n{said}");
    assert!(!scratch.at().join("ca").exists(), "an authority was minted:\n{said}");
}

/// **`--new-fleet` is the answer given in advance**: the same piped run mints
/// the authority, taking the defaults for what it asks after.
#[test]
fn a_piped_run_told_new_fleet_mints_the_authority() {
    let scratch = Scratch::new("guard-new-fleet");
    certificate_in(&scratch.at().join("identity").join("operator"));
    // fleet name; suffixes; keep these; operator name; then quit, do not save.
    let (_, said) =
        interview_with(&scratch, None, &["--new-fleet"], &["", "", "y", "", "q", "n"]);
    assert!(!said.contains("new fleet, or join one?"), "--new-fleet was asked anyway:\n{said}");
    for file in ["root.crt", "root.key", "browser.crt", "browser.key", "suffix"] {
        assert!(scratch.at().join("ca").join(file).is_file(), "no {file}:\n{said}");
    }
}

/// **Editing the hostname and quitting mints nothing.**
#[test]
fn quitting_mints_nothing() {
    let scratch = Scratch::new("quit");
    certificate_in(&scratch.at().join("identity").join("operator"));
    authority_in(&scratch);
    // Hostname; quit; do not save.
    let (_, said) = interview(&scratch, None, &["1", "cedar", "q", "n"]);
    assert!(said.contains("cedar"), "{said}");
    let identity = scratch.at().join("identity");
    let entries: Vec<String> = std::fs::read_dir(&identity)
        .expect("the identity directory")
        .map(|e| e.expect("an entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries, vec!["operator".to_owned()], "quitting left an identity behind");
}

/// **`w` mints the default directory**, which followed the hostname: a seed, a
/// certificate, and a peers file naming the operator.
#[test]
fn w_mints_the_machines_own_identity() {
    let scratch = Scratch::new("mint");
    let operator_key = certificate_in(&scratch.at().join("identity").join("operator"));
    authority_in(&scratch);
    // Hostname; Timezone; write.
    let (ok, said) = interview(&scratch, None, &["1", "cedar", "7", "America/Chicago", "w"]);
    assert!(ok, "{said}");

    let dir = scratch.at().join("identity").join("cedar");
    assert!(said.contains("minted identity"), "{said}");
    assert!(dir.join(rivetos_tls::SEED_FILE).is_file(), "{said}");
    assert!(dir.join(rivetos_tls::CERTIFICATE_FILE).is_file(), "{said}");
    let peers = std::fs::read_to_string(dir.join(rivetos_tls::PEERS_FILE)).expect("peers");
    assert_eq!(peers, format!("{operator_key}\n"));
    let breadcrumb =
        std::fs::read_to_string(scratch.at().join("provision").join(".latest-run.txt"))
            .expect("the breadcrumb");
    assert!(breadcrumb.contains(&format!("identity={}", dir.display())), "{breadcrumb}");
}

/// **An identity whose pin another hostname's record carries is refused at
/// `w`**, naming the record, and nothing is written.
#[test]
fn w_refuses_an_identity_another_machines_record_carries() {
    let scratch = Scratch::new("collision");
    certificate_in(&scratch.at().join("identity").join("operator"));
    authority_in(&scratch);
    let shared = scratch.at().join("identity").join("board");
    let pin = certificate_in(&shared);

    let register_root = scratch.0.join("register");
    let mut register = rivetos_register::Register::empty(&register_root);
    register
        .enroll(rivetos_register::Device {
            id: "1f0532852afc2842".to_owned(),
            name: "board".to_owned(),
            pin: Some(pin),
            ..rivetos_register::Device::default()
        })
        .expect("enrolled");
    rivetos_register::store::save(&register).expect("saved");

    let shared_answer = shared.display().to_string();
    let (_, said) = interview(
        &scratch,
        Some(&register_root),
        &["1", "cedar", "7", "America/Chicago", "9", &shared_answer, "w"],
    );
    assert!(said.contains("refused: NOTHING WAS WRITTEN"), "{said}");
    assert!(said.contains("\"board\""), "{said}");
    assert!(said.contains("1f0532852afc2842"), "{said}");
    assert!(
        !scratch.at().join("provision").join("cedar.provisioning.toml").exists(),
        "a refused `w` wrote the document:\n{said}"
    );
    assert!(!shared.join(rivetos_tls::PEERS_FILE).exists(), "a refused `w` wrote peers");
}
