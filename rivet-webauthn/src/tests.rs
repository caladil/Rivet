//! **Every check, and every refusal, against the software authenticator.**
//!
//! PROVENANCE: self-generated — proves agreement with the author only. See
//! `authenticator.rs`.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use crate::authenticator::{
    AT, ED, ORIGIN, RP_ID, SoftKey, UP, UV, attestation_object, auth_data, cbor, client_data,
    empty_statement,
};
use crate::{
    Algorithm, Credential, Refusal, RelyingParty, UserVerification, browser, store,
    verify_assertion, verify_registration,
};

const RP: RelyingParty<'static> = RelyingParty {
    id: RP_ID,
    origin: ORIGIN,
};
const CHALLENGE: &[u8] = b"a challenge of thirty-two bytes!";
const CRED_ID: &[u8] = b"credential-one";

fn register_with(key: &SoftKey, flags: u8) -> Result<Credential, Refusal> {
    let data = auth_data(RP_ID, flags, 0, Some((CRED_ID, &key.cose())));
    let object = attestation_object("none", empty_statement(), &data);
    let json = client_data("webauthn.create", CHALLENGE, ORIGIN, Some(false));
    verify_registration(RP, UserVerification::Required, CHALLENGE, &json, &object)
}

fn register(key: &SoftKey) -> Credential {
    register_with(key, UP | UV | AT).expect("a good registration")
}

/// One assertion: `(clientDataJSON, authenticatorData, signature)`.
fn assert_with(key: &SoftKey, flags: u8, count: u32) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let data = auth_data(RP_ID, flags, count, None);
    let json = client_data("webauthn.get", CHALLENGE, ORIGIN, None);
    let signature = key.sign(&data, &json);
    (json, data, signature)
}

fn verify(credential: &Credential, parts: &(Vec<u8>, Vec<u8>, Vec<u8>)) -> Result<u32, Refusal> {
    verify_assertion(
        RP,
        UserVerification::Required,
        CHALLENGE,
        credential,
        &credential.id,
        &parts.0,
        &parts.1,
        &parts.2,
    )
    .map(|a| a.sign_count)
}

// --- the good paths -----------------------------------------------------------

#[test]
fn an_es256_credential_registers_and_signs_in() {
    let key = SoftKey::es256();
    let credential = register(&key);
    assert_eq!(credential.algorithm, Algorithm::Es256);
    assert_eq!(credential.id, CRED_ID);
    assert_eq!(credential.public_key.len(), 65);
    assert_eq!(verify(&credential, &assert_with(&key, UP | UV, 0)), Ok(0));
}

#[test]
fn an_ed25519_credential_registers_and_signs_in() {
    let key = SoftKey::ed25519();
    let credential = register(&key);
    assert_eq!(credential.algorithm, Algorithm::Ed25519);
    assert_eq!(credential.public_key.len(), 32);
    assert_eq!(verify(&credential, &assert_with(&key, UP | UV, 7)), Ok(7));
}

/// An extension map after the key (ED) is read past, not refused.
#[test]
fn extension_outputs_after_the_key_are_accounted_for() {
    let key = SoftKey::es256();
    let mut data = auth_data(RP_ID, UP | UV | AT | ED, 0, Some((CRED_ID, &key.cose())));
    data.extend_from_slice(&cbor(&ciborium::Value::Map(vec![(
        ciborium::Value::Text("credProps".into()),
        ciborium::Value::Bool(true),
    )])));
    let object = attestation_object("none", empty_statement(), &data);
    let json = client_data("webauthn.create", CHALLENGE, ORIGIN, None);
    verify_registration(RP, UserVerification::Required, CHALLENGE, &json, &object)
        .expect("extensions are allowed");
}

// --- clientDataJSON -------------------------------------------------------------

fn register_json(json: &[u8]) -> Result<Credential, Refusal> {
    let key = SoftKey::es256();
    let data = auth_data(RP_ID, UP | UV | AT, 0, Some((CRED_ID, &key.cose())));
    let object = attestation_object("none", empty_statement(), &data);
    verify_registration(RP, UserVerification::Required, CHALLENGE, json, &object)
}

#[test]
fn a_get_ceremony_is_not_a_registration() {
    let json = client_data("webauthn.get", CHALLENGE, ORIGIN, None);
    assert!(matches!(register_json(&json), Err(Refusal::WrongType { .. })));
}

#[test]
fn a_challenge_this_machine_did_not_issue_is_refused() {
    let json = client_data("webauthn.create", b"another challenge", ORIGIN, None);
    assert_eq!(register_json(&json), Err(Refusal::ChallengeMismatch));
    // The same bytes with padding are not the same spelling.
    let padded = format!(
        r#"{{"type":"webauthn.create","challenge":"{}=","origin":"{ORIGIN}"}}"#,
        crate::b64url(b"ab")
    );
    let two = RelyingParty { id: RP_ID, origin: ORIGIN };
    let key = SoftKey::es256();
    let data = auth_data(RP_ID, UP | UV | AT, 0, Some((CRED_ID, &key.cose())));
    let object = attestation_object("none", empty_statement(), &data);
    assert_eq!(
        verify_registration(two, UserVerification::Required, b"ab", padded.as_bytes(), &object),
        Err(Refusal::ChallengeMismatch)
    );
}

#[test]
fn every_origin_but_this_machines_is_refused() {
    for origin in [
        "http://dev-01.local",
        "https://dev-01.local:8443",
        "https://dev-01.local.evil.example",
        "https://rivet-01.local",
        "https://DEV-01.local",
    ] {
        let json = client_data("webauthn.create", CHALLENGE, origin, None);
        assert!(
            matches!(register_json(&json), Err(Refusal::OriginMismatch { .. })),
            "{origin}"
        );
    }
}

#[test]
fn a_ceremony_in_a_foreign_frame_is_refused() {
    let json = client_data("webauthn.create", CHALLENGE, ORIGIN, Some(true));
    assert_eq!(register_json(&json), Err(Refusal::CrossOrigin));
    let odd = format!(
        r#"{{"type":"webauthn.create","challenge":"{}","origin":"{ORIGIN}","crossOrigin":"no"}}"#,
        crate::b64url(CHALLENGE)
    );
    assert_eq!(register_json(odd.as_bytes()), Err(Refusal::ClientDataField("crossOrigin")));
}

#[test]
fn client_data_that_is_not_a_json_object_or_lacks_a_field_is_refused() {
    assert_eq!(register_json(b"not json"), Err(Refusal::ClientDataNotJson));
    assert_eq!(register_json(b"[1,2]"), Err(Refusal::ClientDataNotJson));
    let no_origin = format!(
        r#"{{"type":"webauthn.create","challenge":"{}"}}"#,
        crate::b64url(CHALLENGE)
    );
    assert_eq!(register_json(no_origin.as_bytes()), Err(Refusal::ClientDataField("origin")));
    assert_eq!(register_json(br#"{"type":7}"#), Err(Refusal::ClientDataField("type")));
}

// --- the attestation object ------------------------------------------------------

fn register_object(object: &[u8]) -> Result<Credential, Refusal> {
    let json = client_data("webauthn.create", CHALLENGE, ORIGIN, None);
    verify_registration(RP, UserVerification::Required, CHALLENGE, &json, object)
}

#[test]
fn only_attestation_none_is_accepted() {
    let key = SoftKey::es256();
    let data = auth_data(RP_ID, UP | UV | AT, 0, Some((CRED_ID, &key.cose())));
    let packed = attestation_object("packed", empty_statement(), &data);
    assert_eq!(register_object(&packed), Err(Refusal::AttestationFormat("packed".into())));

    let statement = ciborium::Value::Map(vec![(
        ciborium::Value::Text("alg".into()),
        ciborium::Value::Integer((-7).into()),
    )]);
    let stuffed = attestation_object("none", statement, &data);
    assert_eq!(register_object(&stuffed), Err(Refusal::AttestationStatementNotEmpty));

    let mut trailing = attestation_object("none", empty_statement(), &data);
    trailing.push(0);
    assert!(matches!(register_object(&trailing), Err(Refusal::AttestationObject(_))));
    assert!(matches!(register_object(b"\xff\xff"), Err(Refusal::AttestationObject(_))));
    let no_data = cbor(&ciborium::Value::Map(vec![(
        ciborium::Value::Text("fmt".into()),
        ciborium::Value::Text("none".into()),
    )]));
    assert!(matches!(register_object(&no_data), Err(Refusal::AttestationObject(_))));
}

#[test]
fn a_credential_for_another_relying_party_is_refused() {
    let key = SoftKey::es256();
    let data = auth_data("rivet-01.local", UP | UV | AT, 0, Some((CRED_ID, &key.cose())));
    let object = attestation_object("none", empty_statement(), &data);
    assert_eq!(register_object(&object), Err(Refusal::RpIdHashMismatch));
}

#[test]
fn presence_and_verification_are_both_required() {
    let key = SoftKey::es256();
    assert_eq!(register_with(&key, UV | AT), Err(Refusal::UserNotPresent));
    assert_eq!(register_with(&key, UP | AT), Err(Refusal::UserNotVerified));
    // The same bytes pass when the caller does not require UV, which is the
    // only thing the parameter changes.
    let data = auth_data(RP_ID, UP | AT, 0, Some((CRED_ID, &key.cose())));
    let object = attestation_object("none", empty_statement(), &data);
    let json = client_data("webauthn.create", CHALLENGE, ORIGIN, None);
    assert!(verify_registration(RP, UserVerification::NotRequired, CHALLENGE, &json, &object).is_ok());
}

#[test]
fn a_registration_without_a_credential_is_refused() {
    let data = auth_data(RP_ID, UP | UV, 0, None);
    let object = attestation_object("none", empty_statement(), &data);
    assert_eq!(register_object(&object), Err(Refusal::NoAttestedCredential));
}

#[test]
fn authenticator_data_that_does_not_account_for_its_bytes_is_refused() {
    let key = SoftKey::es256();
    let mut data = auth_data(RP_ID, UP | UV | AT, 0, Some((CRED_ID, &key.cose())));
    data.push(0xa0);
    let object = attestation_object("none", empty_statement(), &data);
    assert!(matches!(register_object(&object), Err(Refusal::AuthenticatorData(_))));

    let short = auth_data(RP_ID, UP | UV | AT, 0, None);
    let object = attestation_object("none", empty_statement(), &short[..20]);
    assert!(matches!(register_object(&object), Err(Refusal::AuthenticatorData(_))));

    let truncated = auth_data(RP_ID, UP | UV | AT, 0, None);
    let object = attestation_object("none", empty_statement(), &truncated);
    assert!(matches!(register_object(&object), Err(Refusal::AuthenticatorData(_))));
}

#[test]
fn an_empty_credential_id_is_refused() {
    let key = SoftKey::es256();
    let data = auth_data(RP_ID, UP | UV | AT, 0, Some((b"", &key.cose())));
    let object = attestation_object("none", empty_statement(), &data);
    assert_eq!(register_object(&object), Err(Refusal::CredentialIdLength(0)));
}

fn register_key(key: &ciborium::Value) -> Result<Credential, Refusal> {
    let data = auth_data(RP_ID, UP | UV | AT, 0, Some((CRED_ID, &cbor(key))));
    register_object(&attestation_object("none", empty_statement(), &data))
}

#[test]
fn only_es256_and_ed25519_keys_are_accepted() {
    use ciborium::Value as V;
    let int = |i: i64| V::Integer(i.into());
    // RS256, the other algorithm browsers commonly offer.
    let rsa = V::Map(vec![(int(1), int(3)), (int(3), int(-257))]);
    assert_eq!(register_key(&rsa), Err(Refusal::UnsupportedAlgorithm(-257)));
    // ES256 on the wrong curve, and with the wrong key type.
    let p384 = V::Map(vec![
        (int(1), int(2)),
        (int(3), int(-7)),
        (int(-1), int(2)),
        (int(-2), V::Bytes(vec![1; 32])),
        (int(-3), V::Bytes(vec![1; 32])),
    ]);
    assert!(matches!(register_key(&p384), Err(Refusal::PublicKey(_))));
    let okp = V::Map(vec![(int(1), int(1)), (int(3), int(-7)), (int(-1), int(1))]);
    assert!(matches!(register_key(&okp), Err(Refusal::PublicKey(_))));
    // An x that is not 32 bytes.
    let short = V::Map(vec![
        (int(1), int(1)),
        (int(3), int(-8)),
        (int(-1), int(6)),
        (int(-2), V::Bytes(vec![1; 31])),
    ]);
    assert!(matches!(register_key(&short), Err(Refusal::PublicKey(_))));
    // A label given twice.
    let twice = V::Map(vec![
        (int(1), int(1)),
        (int(3), int(-8)),
        (int(3), int(-8)),
        (int(-1), int(6)),
        (int(-2), V::Bytes(vec![1; 32])),
    ]);
    assert!(matches!(register_key(&twice), Err(Refusal::PublicKey(_))));
    assert!(matches!(register_key(&V::Text("key".into())), Err(Refusal::PublicKey(_))));
}

// --- assertions -----------------------------------------------------------------

#[test]
fn a_signature_over_other_bytes_or_by_another_key_is_refused() {
    let key = SoftKey::es256();
    let credential = register(&key);
    let (json, data, signature) = assert_with(&key, UP | UV, 0);
    let mut tampered = data.clone();
    tampered[36] = 1; // the counter, after signing
    assert_eq!(
        verify(&credential, &(json.clone(), tampered, signature.clone())),
        Err(Refusal::SignatureInvalid)
    );
    let other = SoftKey::es256();
    let (json2, data2, signature2) = assert_with(&other, UP | UV, 0);
    assert_eq!(
        verify(&credential, &(json2, data2, signature2)),
        Err(Refusal::SignatureInvalid)
    );
    // The client data is covered too.
    let mut json3 = json;
    json3.extend_from_slice(b" ");
    assert_eq!(
        verify(&credential, &(json3, data, signature)),
        Err(Refusal::SignatureInvalid)
    );
}

#[test]
fn an_ed25519_signature_by_another_key_is_refused() {
    let key = SoftKey::ed25519();
    let credential = register(&key);
    let other = SoftKey::ed25519();
    assert_eq!(
        verify(&credential, &assert_with(&other, UP | UV, 1)),
        Err(Refusal::SignatureInvalid)
    );
}

#[test]
fn the_counter_must_increase_once_either_side_is_non_zero() {
    let key = SoftKey::es256();
    let mut credential = register(&key);
    credential.sign_count = 5;
    for presented in [0, 3, 5] {
        assert_eq!(
            verify(&credential, &assert_with(&key, UP | UV, presented)),
            Err(Refusal::CounterRegression { stored: 5, presented }),
            "{presented}"
        );
    }
    assert_eq!(verify(&credential, &assert_with(&key, UP | UV, 6)), Ok(6));
    credential.sign_count = 0;
    assert_eq!(verify(&credential, &assert_with(&key, UP | UV, 0)), Ok(0));
    assert_eq!(verify(&credential, &assert_with(&key, UP | UV, 1)), Ok(1));
}

#[test]
fn an_assertion_is_refused_for_each_ceremony_check() {
    let key = SoftKey::es256();
    let credential = register(&key);

    assert_eq!(
        verify(&credential, &assert_with(&key, UP, 1)),
        Err(Refusal::UserNotVerified)
    );
    assert_eq!(
        verify(&credential, &assert_with(&key, UV, 1)),
        Err(Refusal::UserNotPresent)
    );

    let data = auth_data("rivet-01.local", UP | UV, 1, None);
    let json = client_data("webauthn.get", CHALLENGE, ORIGIN, None);
    let signature = key.sign(&data, &json);
    assert_eq!(verify(&credential, &(json, data, signature)), Err(Refusal::RpIdHashMismatch));

    let data = auth_data(RP_ID, UP | UV, 1, None);
    let json = client_data("webauthn.create", CHALLENGE, ORIGIN, None);
    let signature = key.sign(&data, &json);
    assert!(matches!(
        verify(&credential, &(json, data, signature)),
        Err(Refusal::WrongType { .. })
    ));

    let data = auth_data(RP_ID, UP | UV, 1, None);
    let json = client_data("webauthn.get", b"stale", ORIGIN, None);
    let signature = key.sign(&data, &json);
    assert_eq!(verify(&credential, &(json, data, signature)), Err(Refusal::ChallengeMismatch));

    let parts = assert_with(&key, UP | UV, 1);
    assert_eq!(
        verify_assertion(
            RP,
            UserVerification::Required,
            CHALLENGE,
            &credential,
            b"another-credential",
            &parts.0,
            &parts.1,
            &parts.2,
        ),
        Err(Refusal::CredentialMismatch)
    );
}

// --- the store ------------------------------------------------------------------

#[test]
fn the_store_round_trips() {
    let one = store::Stored {
        credential: register(&SoftKey::es256()),
        enrolled: 1_790_000_000,
        rp_id: None,
    };
    let mut two = store::Stored {
        credential: register(&SoftKey::ed25519()),
        enrolled: 1_790_000_100,
        rp_id: Some("dev-01.home-fleet.home.arpa".to_string()),
    };
    two.credential.id = b"credential-two".to_vec();
    two.credential.sign_count = 42;
    let text = store::render(&[one.clone(), two.clone()]);
    assert!(text.starts_with(store::HEADER));
    assert_eq!(store::parse(&text), Ok(vec![one, two]));
    assert_eq!(store::parse(""), Ok(Vec::new()));
}

/// **A store written before the RP ID was recorded still reads**, as
/// passkeys with no name of their own, and renders back byte for byte — so
/// a counter update never rewrites such a line into a name it was not given.
///
/// PROVENANCE: the passkey line is the one dev-01's proxy wrote when a
/// passkey was enrolled from Chrome on Windows (a public key, nothing secret),
/// under the old header.
#[test]
fn a_store_written_before_rp_ids_reads_and_renders_unchanged() {
    let old = "# rivet-proxyd passkeys: passkey <id> <algorithm> <public key> <counter> <enrolled>\n\
               passkey fAbOmZ85O5HvF6gegifR4w es256 BGNEoOCgId5XDI7EjIdHuaEYKMixYDNrDFSN8er6i6zsBtL4Ef54DA-xR8Iuu6xjv449TJwHhkGuQHOorLrLs-A 0 1790730159\n";
    let stored = store::parse(old).expect("the old format reads");
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].rp_id, None);
    assert_eq!(stored[0].enrolled_at("dev-01.local"), "dev-01.local");
    let line = old.lines().nth(1).expect("a line");
    assert_eq!(store::render(&stored).lines().nth(1), Some(line));

    let named = store::Stored {
        rp_id: Some("dev-01.home-fleet.home.arpa".to_string()),
        ..stored[0].clone()
    };
    assert_eq!(named.enrolled_at("dev-01.local"), "dev-01.home-fleet.home.arpa");
}

#[test]
fn a_store_line_that_does_not_read_refuses_the_whole_store() {
    let good = store::render(&[store::Stored {
        credential: register(&SoftKey::ed25519()),
        enrolled: 1,
        rp_id: None,
    }]);
    let line = good.lines().nth(1).expect("a line");
    for bad in [
        "passkey a b c d".to_string(),
        line.replacen("passkey", "key", 1),
        line.replacen("ed25519", "rs256", 1),
        line.replacen(" 1", " one", 1),
        format!("{line}\n{line}"),
        line.replace(line.split(' ').nth(3).expect("key"), "AAAA"),
        format!("{line} Dev-01.local"),
        format!("{line} dev-01.local:443"),
        format!("{line} dev-01.local extra"),
        format!("{line} {}", "a".repeat(254)),
    ] {
        let text = format!("{}\n{bad}\n", store::HEADER);
        assert!(store::parse(&text).is_err(), "{bad}");
    }
}

// --- what the pages send --------------------------------------------------------

#[test]
fn the_pages_bodies_are_read_strictly() {
    let body = br#"{"clientDataJSON":"e30","attestationObject":"oA"}"#;
    let reg = browser::registration(body).expect("readable");
    assert_eq!(reg.client_data_json, b"{}");
    assert!(browser::registration(br#"{"clientDataJSON":"e30="}"#).is_err());
    assert!(browser::registration(br#"{"clientDataJSON":"e30=","attestationObject":"oA"}"#).is_err());
    assert!(browser::assertion(br#"{"id":"AA","clientDataJSON":"e30","authenticatorData":"AA"}"#).is_err());
    assert_eq!(browser::enrollment_code(b""), Ok(None));
    assert_eq!(browser::enrollment_code(b"{}"), Ok(None));
    assert_eq!(
        browser::enrollment_code(br#"{"code":"ABCDE-FGHJK"}"#),
        Ok(Some("ABCDE-FGHJK".into()))
    );
    assert!(browser::enrollment_code(br#"{"code":5}"#).is_err());
    assert!(browser::enrollment_code(b"code").is_err());
}
