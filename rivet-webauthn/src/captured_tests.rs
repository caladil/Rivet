//! **A ceremony captured from a real browser**, verified.
//!
//! PROVENANCE: captured from a real browser — the bytes and where they came
//! from are in `captured`.
//!
//! Two things in it the other vectors do not have: Chrome's `get`
//! `clientDataJSON` carries an extra key telling relying parties not to compare
//! it against a template, and the authenticator keeps no signature counter
//! (zero on both ceremonies), with the backup-eligible and backed-up flags set.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use crate::captured::{
    CREATE_ATTESTATION, CREATE_CHALLENGE, CREATE_CLIENT_DATA, CREDENTIAL_ID, GET_AUTH_DATA,
    GET_CHALLENGE, GET_CLIENT_DATA, GET_SIGNATURE, ORIGIN, RP_ID,
};
use crate::{
    Algorithm, Refusal, RelyingParty, UserVerification, from_b64url, verify_assertion,
    verify_registration,
};

const RP: RelyingParty<'static> = RelyingParty {
    id: RP_ID,
    origin: ORIGIN,
};

fn b(text: &str) -> Vec<u8> {
    from_b64url(text).expect("base64url")
}

fn registered() -> crate::Credential {
    verify_registration(
        RP,
        UserVerification::Required,
        &b(CREATE_CHALLENGE),
        &b(CREATE_CLIENT_DATA),
        &b(CREATE_ATTESTATION),
    )
    .expect("the captured registration verifies")
}

#[test]
fn the_captured_registration_yields_an_es256_credential_with_its_id() {
    let credential = registered();
    assert_eq!(credential.algorithm, Algorithm::Es256);
    assert_eq!(credential.id, b(CREDENTIAL_ID));
    assert_eq!(credential.public_key.len(), 65);
    assert_eq!(credential.sign_count, 0);
}

#[test]
fn the_captured_assertion_verifies_against_the_captured_registration() {
    let credential = registered();
    let asserted = verify_assertion(
        RP,
        UserVerification::Required,
        &b(GET_CHALLENGE),
        &credential,
        &b(CREDENTIAL_ID),
        &b(GET_CLIENT_DATA),
        &b(GET_AUTH_DATA),
        &b(GET_SIGNATURE),
    )
    .expect("the captured assertion verifies");
    assert_eq!(asserted.sign_count, 0);
}

#[test]
fn the_captured_assertion_is_refused_with_one_signature_byte_changed() {
    let credential = registered();
    let mut signature = b(GET_SIGNATURE);
    let last = signature.len() - 1;
    signature[last] ^= 0x01;
    let refused = verify_assertion(
        RP,
        UserVerification::Required,
        &b(GET_CHALLENGE),
        &credential,
        &b(CREDENTIAL_ID),
        &b(GET_CLIENT_DATA),
        &b(GET_AUTH_DATA),
        &signature,
    );
    assert_eq!(refused, Err(Refusal::SignatureInvalid));
}

#[test]
fn the_captured_assertion_is_refused_for_another_challenge_or_another_name() {
    let credential = registered();
    let other_challenge = verify_assertion(
        RP,
        UserVerification::Required,
        &b(CREATE_CHALLENGE),
        &credential,
        &b(CREDENTIAL_ID),
        &b(GET_CLIENT_DATA),
        &b(GET_AUTH_DATA),
        &b(GET_SIGNATURE),
    );
    assert!(other_challenge.is_err(), "{other_challenge:?}");

    let elsewhere = RelyingParty {
        id: "dev-01.home-fleet.home.arpa",
        origin: "https://dev-01.home-fleet.home.arpa",
    };
    let other_name = verify_assertion(
        elsewhere,
        UserVerification::Required,
        &b(GET_CHALLENGE),
        &credential,
        &b(CREDENTIAL_ID),
        &b(GET_CLIENT_DATA),
        &b(GET_AUTH_DATA),
        &b(GET_SIGNATURE),
    );
    assert!(other_name.is_err(), "{other_name:?}");
}
