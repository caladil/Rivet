//! What a fleet-root machine issues, and when, over a fake kernel holding real
//! certificates. Every root and leaf here is **self-generated** by the test
//! that uses it: a counter stands in for `getrandom(2)` and the keys come from
//! `ring`'s generator. The fake does not sign; it records the order and puts
//! placeholder text where the helper would have written.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::journal::Journal;
use crate::service::IMAGE_DEFINED;
use crate::testkernel::{FakeKernel, Script};
use rivet_core::fleet_root::Request;

fn journal() -> Journal {
    Journal::new("b".to_string(), 1, crate::supervise::ACTOR.to_string())
}

fn counting() -> impl FnMut(usize) -> Result<Vec<u8>, String> {
    let mut next = 11u8;
    move |len| {
        Ok((0..len)
            .map(|_| {
                next = next.wrapping_add(43);
                next
            })
            .collect())
    }
}

/// A fleet's root, as it rests on a fleet server.
struct Fleet {
    certificate: String,
    key: String,
    request: String,
    names: Vec<String>,
}

fn fleet(name: &str) -> Fleet {
    let request = Request::new(
        name,
        "dev-01",
        &["local".to_string(), format!("{name}.home.arpa")],
    )
    .expect("a legal request");
    let minted = rivet_fleet_root::mint(&request, &mut counting(), 0).expect("mints");
    Fleet {
        certificate: minted.root.certificate,
        key: minted.root.key,
        request: request.render(),
        names: minted.names,
    }
}

/// A leaf this fleet's root issues today, with every field.
fn leaf_of(f: &Fleet) -> String {
    rivet_fleet_root::judge::issue_from_root(&f.certificate, &f.key, &f.names, &mut counting(), 0)
        .expect("issues")
        .certificate
}

/// A leaf this fleet's root signs WITHOUT the standard fields: `rcgen`'s
/// defaults, which write none of them.
fn bare_leaf_of(f: &Fleet) -> String {
    let key = rcgen::KeyPair::from_pem(&f.key).expect("the key reads");
    let issuer = rcgen::Issuer::from_ca_cert_pem(&f.certificate, key).expect("an issuer");
    let leaf_key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("a key");
    let mut params = rcgen::CertificateParams::new(f.names.clone()).expect("params");
    params.not_before = rcgen::date_time_ymd(2026, 1, 1);
    params.not_after = rcgen::date_time_ymd(2031, 1, 1);
    params.signed_by(&leaf_key, &issuer).expect("signs").pem()
}

fn name(text: &str) -> ServiceName {
    ServiceName::new(text).expect("a valid fixture name")
}

fn data(app: &str) -> String {
    service::data_dir(service::BUILT_IN_VOLUME, &name(app))
}

fn def(app: &str, origin: Origin, material: Vec<Material>) -> ServiceDef {
    ServiceDef {
        name: app.to_string(),
        origin,
        volume: Some(service::BUILT_IN_VOLUME.to_string()),
        material,
        program: crate::kernel::Program {
            path: String::new(),
            args: Vec::new(),
            env: Vec::new(),
            identity: Some(service::app_identity(&name(app))),
            confinement: None,
            groups: Vec::new(),
            capabilities: Vec::new(),
        },
        ..IMAGE_DEFINED
    }
}

fn tls_app(app: &str) -> ServiceDef {
    def(app, Origin::Application, vec![Material::FleetTls])
}

/// A machine holding `f` as its root.
fn machine(f: &Fleet) -> FakeKernel {
    let mut k = FakeKernel::new();
    k.files_mut()
        .insert(root_certificate_path(), Script::always(Ok(f.certificate.clone())));
    k.exists_mut()
        .insert(root_certificate_path(), Script::always(Ok(true)));
    k.files_mut().insert(
        format!("{}/{MINTED_FOR_FILE}", rivet_core::state::SECRETS),
        Script::always(Ok(f.request.clone())),
    );
    k
}

/// `app` holding `certificate`, a key, and `root` as its root file.
fn holding(k: &mut FakeKernel, app: &str, certificate: &str, root: &str) {
    let dir = data(app);
    k.files_mut()
        .insert(format!("{dir}/tls-cert"), Script::always(Ok(certificate.to_string())));
    k.exists_mut()
        .insert(format!("{dir}/tls-key"), Script::always(Ok(true)));
    k.files_mut()
        .insert(format!("{dir}/tls-root"), Script::always(Ok(root.to_string())));
}

fn record_with<'a>(k: &'a FakeKernel, svc: &str) -> &'a crate::journal::Record {
    k.records
        .iter()
        .find(|r| r.svc == svc)
        .unwrap_or_else(|| panic!("nothing was journalled for {svc}: {:?}", k.records))
}

fn field<'a>(record: &'a crate::journal::Record, key: &str) -> Option<&'a str> {
    record.fields.iter().find_map(|(k, v)| match v {
        Value::Str(s) if k == key => Some(s.as_str()),
        _ => None,
    })
}

/// **The supervisor never reads the root's key**: every order names it by
/// path, and no read of it was made in this process.
fn never_read_the_key(k: &FakeKernel) {
    assert!(
        !k.reads.contains(&root_key_path()),
        "the supervisor read the root's key: {:?}",
        k.reads
    );
    for order in &k.issues {
        assert_eq!(order.root_key, root_key_path());
    }
}

/// **At install, an application with no certificate is issued one**, into its
/// own data directory, owned by its own uid, carrying the machine's names.
#[test]
fn an_application_is_issued_its_certificate_at_install() {
    let f = fleet("dev-fleet");
    let mut k = machine(&f);
    let app = tls_app("aura-controller");

    converge(&mut k, &mut journal(), std::slice::from_ref(&app), Occasion::Install, &[]);

    assert_eq!(k.issues.len(), 1, "{:?}", k.issues);
    let order = &k.issues[0];
    assert_eq!(order.names, f.names);
    let id = app.program.identity.expect("an identity");
    assert_eq!(
        order.targets,
        vec![IssueTarget {
            dir: data("aura-controller"),
            certificate: "tls-cert".to_string(),
            key: "tls-key".to_string(),
            root: Some("tls-root".to_string()),
            uid: id.uid,
            gid: id.gid,
        }]
    );
    let record = record_with(&k, "aura-controller");
    assert_eq!(record.sev, Severity::Info);
    assert_eq!(field(record, "reason"), Some("the certificate is missing"));
    never_read_the_key(&k);
}

/// **At boot, each trigger issues**: missing, a foreign issuer (journalled by
/// name), near its end, and lacking the standard fields.
#[test]
fn every_trigger_issues_at_boot() {
    let f = fleet("dev-fleet");
    let workstation = fleet("workstation");
    let five_years_less_thirty_days = 1_924_992_000 - 30 * 86_400;

    type Arrange<'a> = Box<dyn Fn(&mut FakeKernel) + 'a>;
    let cases: Vec<(&str, Arrange<'_>)> = vec![
        ("missing", Box::new(|_k: &mut FakeKernel| {})),
        (
            "foreign",
            Box::new(|k: &mut FakeKernel| {
                holding(k, "aura-controller", &leaf_of(&workstation), &workstation.certificate);
            }),
        ),
        (
            "near expiry",
            Box::new(|k: &mut FakeKernel| {
                holding(k, "aura-controller", &leaf_of(&f), &f.certificate);
                k.wall_ms = Some(five_years_less_thirty_days * 1000);
            }),
        ),
        (
            "lacking fields",
            Box::new(|k: &mut FakeKernel| {
                holding(k, "aura-controller", &bare_leaf_of(&f), &f.certificate);
            }),
        ),
    ];
    for (case, arrange) in cases {
        let mut k = machine(&f);
        arrange(&mut k);
        converge(
            &mut k,
            &mut journal(),
            &[tls_app("aura-controller")],
            Occasion::Boot,
            &[],
        );
        assert_eq!(k.issues.len(), 1, "{case}: {:?}", k.records);
        let record = record_with(&k, "aura-controller");
        let reason = field(record, "reason").unwrap_or_default();
        match case {
            "missing" => assert!(reason.contains("missing"), "{reason}"),
            "foreign" => {
                assert!(reason.contains("CN=workstation"), "{reason}");
                assert_eq!(field(record, "old_issuer"), Some("CN=workstation"));
            }
            "near expiry" => assert!(reason.contains("ninety days"), "{reason}"),
            _ => assert!(reason.contains("authority key identifier"), "{reason}"),
        }
        never_read_the_key(&k);
    }
}

/// 2031-01-01, the end of a leaf issued at the floor, as `date -u -d
/// 2031-01-01 +%s` says.
#[test]
fn the_near_expiry_fixture_is_the_end_of_a_leaf_issued_at_the_floor() {
    assert_eq!(1_924_992_000u64, 1_767_225_600 + (365 * 4 + 366) * 86_400);
}

/// **A current certificate is left alone**, and nothing is said about it.
#[test]
fn a_current_certificate_is_left_alone() {
    let f = fleet("dev-fleet");
    let mut k = machine(&f);
    holding(&mut k, "aura-controller", &leaf_of(&f), &f.certificate);
    converge(&mut k, &mut journal(), &[tls_app("aura-controller")], Occasion::Boot, &[]);
    assert!(k.issues.is_empty(), "{:?}", k.issues);
    assert!(k.records.is_empty(), "{:?}", k.records);
}

/// **A current certificate beside a root file that is not this machine's root
/// is re-issued**, so the application trusts the root that signs its callers.
#[test]
fn a_stale_root_file_is_replaced() {
    let f = fleet("dev-fleet");
    let workstation = fleet("workstation");
    let mut k = machine(&f);
    holding(&mut k, "aura-controller", &leaf_of(&f), &workstation.certificate);
    converge(&mut k, &mut journal(), &[tls_app("aura-controller")], Occasion::Boot, &[]);
    assert_eq!(k.issues.len(), 1);
}

/// **A workstation-root machine issues nothing**, reads nothing in
/// `/state/secrets`, and says nothing: `cargo rivet secrets` is still how its
/// material arrives.
#[test]
fn a_workstation_root_machine_issues_nothing() {
    let mut k = FakeKernel::new();
    let mut proxy = def("rivet-proxyd", Origin::Image, Vec::new());
    proxy.enabled = true;
    converge(
        &mut k,
        &mut journal(),
        &[tls_app("aura-controller"), proxy],
        Occasion::Boot,
        &[],
    );
    assert!(k.issues.is_empty(), "{:?}", k.issues);
    assert!(k.records.is_empty(), "{:?}", k.records);
    assert!(
        k.reads.iter().all(|p| !p.starts_with(rivet_core::state::SECRETS)),
        "{:?}",
        k.reads
    );
    assert!(!issues_here(&mut k));
}

/// An application that declares no material is never judged.
#[test]
fn an_application_declaring_no_material_is_not_judged() {
    let f = fleet("dev-fleet");
    let mut k = machine(&f);
    converge(
        &mut k,
        &mut journal(),
        &[def("plain", Origin::Application, Vec::new())],
        Occasion::Boot,
        &[],
    );
    assert!(k.issues.is_empty());
}

/// **The proxy's and the fleet service's shared leaf is re-issued at boot when
/// it lacks the standard fields**, as one leaf: resting in `/state/secrets`
/// beside the root, owned by root, and handed to each program under its own
/// names and uid — exactly where the first boot put it.
#[test]
fn the_shared_leaf_is_reissued_at_boot_when_it_lacks_the_fields() {
    let f = fleet("dev-fleet");
    let mut k = machine(&f);
    let proxy = def("rivet-proxyd", Origin::Image, Vec::new());
    let fleetd = def("rivet-fleetd", Origin::Application, Vec::new());
    let bare = bare_leaf_of(&f);
    for (app, certificate) in [("rivet-proxyd", "tls-cert"), ("rivet-fleetd", "server.crt")] {
        k.exists_mut().insert(data(app), Script::always(Ok(true)));
        k.files_mut().insert(
            format!("{}/{certificate}", data(app)),
            Script::always(Ok(bare.clone())),
        );
    }

    converge(
        &mut k,
        &mut journal(),
        &[proxy.clone(), fleetd.clone()],
        Occasion::Boot,
        &[],
    );

    assert_eq!(k.issues.len(), 1, "one leaf for both: {:?}", k.issues);
    let order = &k.issues[0];
    assert_eq!(order.names, f.names, "the names must not change, or a passkey stops working");
    let proxy_id = proxy.program.identity.expect("an identity");
    let fleetd_id = fleetd.program.identity.expect("an identity");
    assert_eq!(
        order.targets,
        vec![
            IssueTarget {
                dir: rivet_core::state::SECRETS.to_string(),
                certificate: SERVER_CERTIFICATE_FILE.to_string(),
                key: SERVER_KEY_FILE.to_string(),
                root: None,
                uid: 0,
                gid: 0,
            },
            IssueTarget {
                dir: data("rivet-proxyd"),
                certificate: "tls-cert".to_string(),
                key: "tls-key".to_string(),
                root: Some("tls-root".to_string()),
                uid: proxy_id.uid,
                gid: proxy_id.gid,
            },
            IssueTarget {
                dir: data("rivet-fleetd"),
                certificate: "server.crt".to_string(),
                key: "server.key".to_string(),
                root: Some("ca.crt".to_string()),
                uid: fleetd_id.uid,
                gid: fleetd_id.gid,
            },
        ]
    );
    never_read_the_key(&k);
}

/// **A current shared leaf is left alone at boot, and the shared leaf is never
/// judged at an install**: the proxy is running then and reads its leaf only
/// at its start.
#[test]
fn the_shared_leaf_is_left_alone_when_current_and_at_an_install() {
    let f = fleet("dev-fleet");
    let proxy = def("rivet-proxyd", Origin::Image, Vec::new());

    let mut k = machine(&f);
    k.exists_mut()
        .insert(data("rivet-proxyd"), Script::always(Ok(true)));
    k.files_mut().insert(
        format!("{}/tls-cert", data("rivet-proxyd")),
        Script::always(Ok(leaf_of(&f))),
    );
    converge(&mut k, &mut journal(), std::slice::from_ref(&proxy), Occasion::Boot, &[]);
    assert!(k.issues.is_empty(), "{:?}", k.issues);

    let mut k = machine(&f);
    k.exists_mut()
        .insert(data("rivet-proxyd"), Script::always(Ok(true)));
    converge(&mut k, &mut journal(), &[proxy], Occasion::Install, &[]);
    assert!(k.issues.is_empty(), "{:?}", k.issues);
}

/// **A helper that fails is journalled as an error naming why**, and nothing
/// else is claimed.
#[test]
fn a_failed_issuance_is_journalled_as_an_error() {
    let f = fleet("dev-fleet");
    let mut k = machine(&f);
    k.issue_results = Script::always(Ok(Err("simulated: getrandom refused".to_string())));
    converge(&mut k, &mut journal(), &[tls_app("aura-controller")], Occasion::Install, &[]);
    let record = record_with(&k, "aura-controller");
    assert_eq!(record.sev, Severity::Error);
    assert_eq!(field(record, "detail"), Some("simulated: getrandom refused"));
}

/// **A running application is told it reads the new leaf at its next start**:
/// an install restarts nothing.
#[test]
fn a_running_application_is_not_restarted_for_its_new_leaf() {
    let f = fleet("dev-fleet");
    let mut k = machine(&f);
    converge(
        &mut k,
        &mut journal(),
        &[tls_app("aura-controller")],
        Occasion::Install,
        &["aura-controller".to_string()],
    );
    let record = record_with(&k, "aura-controller");
    assert!(record.msg.contains("nothing was restarted"), "{}", record.msg);
    assert!(k.signals.is_empty());
}
