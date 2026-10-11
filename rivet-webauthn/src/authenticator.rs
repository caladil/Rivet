//! **A software authenticator, for this crate's own tests.**
//!
//! PROVENANCE: everything it produces is SELF-GENERATED — it proves this
//! crate agrees with its own author about WebAuthn's byte layouts, and nothing
//! more (`AGENTS.md` §4). The W3C vectors in `w3c_tests.rs` are the check
//! against somebody else's reading, and a browser capture from
//! `examples/probes/passkey-probe` is the check against a real authenticator.
//!
//! Its value is that it can produce every WRONG ceremony on purpose — a bad
//! flag, a foreign RP ID, a stale counter, a signature over other bytes —
//! which no fixed vector can.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use ring::signature::KeyPair as _;

pub const RP_ID: &str = "dev-01.local";
pub const ORIGIN: &str = "https://dev-01.local";

pub const UP: u8 = 0x01;
pub const UV: u8 = 0x04;
pub const AT: u8 = 0x40;
pub const ED: u8 = 0x80;

pub enum SoftKey {
    Es256(ring::signature::EcdsaKeyPair),
    Ed25519(ring::signature::Ed25519KeyPair),
}

impl SoftKey {
    pub fn es256() -> Self {
        let rng = ring::rand::SystemRandom::new();
        let alg = &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING;
        let pkcs8 = ring::signature::EcdsaKeyPair::generate_pkcs8(alg, &rng).expect("a key");
        Self::Es256(
            ring::signature::EcdsaKeyPair::from_pkcs8(alg, pkcs8.as_ref(), &rng).expect("a pair"),
        )
    }

    pub fn ed25519() -> Self {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("a key");
        Self::Ed25519(ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("a pair"))
    }

    /// The public key as COSE_Key CBOR.
    pub fn cose(&self) -> Vec<u8> {
        use ciborium::Value as V;
        let int = |i: i64| V::Integer(i.into());
        let map = match self {
            Self::Es256(pair) => {
                let point = pair.public_key().as_ref();
                V::Map(vec![
                    (int(1), int(2)),
                    (int(3), int(-7)),
                    (int(-1), int(1)),
                    (int(-2), V::Bytes(point[1..33].to_vec())),
                    (int(-3), V::Bytes(point[33..65].to_vec())),
                ])
            }
            Self::Ed25519(pair) => V::Map(vec![
                (int(1), int(1)),
                (int(3), int(-8)),
                (int(-1), int(6)),
                (int(-2), V::Bytes(pair.public_key().as_ref().to_vec())),
            ]),
        };
        cbor(&map)
    }

    pub fn sign(&self, auth_data: &[u8], client_data_json: &[u8]) -> Vec<u8> {
        let mut message = auth_data.to_vec();
        message.extend_from_slice(
            ring::digest::digest(&ring::digest::SHA256, client_data_json).as_ref(),
        );
        match self {
            Self::Es256(pair) => pair
                .sign(&ring::rand::SystemRandom::new(), &message)
                .expect("a signature")
                .as_ref()
                .to_vec(),
            Self::Ed25519(pair) => pair.sign(&message).as_ref().to_vec(),
        }
    }
}

pub fn cbor(value: &ciborium::Value) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::into_writer(value, &mut out).expect("CBOR");
    out
}

/// Authenticator data for `rp_id`, with `flags`, `count`, and — when given —
/// an attested credential `(id, cose key)`.
pub fn auth_data(rp_id: &str, flags: u8, count: u32, attested: Option<(&[u8], &[u8])>) -> Vec<u8> {
    let mut out = ring::digest::digest(&ring::digest::SHA256, rp_id.as_bytes())
        .as_ref()
        .to_vec();
    out.push(flags);
    out.extend_from_slice(&count.to_be_bytes());
    if let Some((id, key)) = attested {
        out.extend_from_slice(&[0u8; 16]);
        out.extend_from_slice(&u16::try_from(id.len()).expect("short").to_be_bytes());
        out.extend_from_slice(id);
        out.extend_from_slice(key);
    }
    out
}

/// `clientDataJSON` as a browser writes it.
pub fn client_data(kind: &str, challenge: &[u8], origin: &str, cross_origin: Option<bool>) -> Vec<u8> {
    let mut text = format!(
        r#"{{"type":"{kind}","challenge":"{}","origin":"{origin}""#,
        crate::b64url(challenge)
    );
    if let Some(cross) = cross_origin {
        text.push_str(&format!(r#","crossOrigin":{cross}"#));
    }
    text.push('}');
    text.into_bytes()
}

/// An attestation object.
pub fn attestation_object(fmt: &str, statement: ciborium::Value, auth_data: &[u8]) -> Vec<u8> {
    use ciborium::Value as V;
    cbor(&V::Map(vec![
        (V::Text("fmt".into()), V::Text(fmt.into())),
        (V::Text("attStmt".into()), statement),
        (V::Text("authData".into()), V::Bytes(auth_data.to_vec())),
    ]))
}

pub fn empty_statement() -> ciborium::Value {
    ciborium::Value::Map(Vec::new())
}
