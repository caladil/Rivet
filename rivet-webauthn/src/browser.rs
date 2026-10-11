//! **What the proxy's pages send**, read strictly.
//!
//! The pages post JSON whose binary fields are base64url without padding —
//! the spelling [`crate::b64url`] produces and the pages' own script writes.
//! A field that is missing, not a string, or not that spelling is refused by
//! name, so a page and this reader cannot disagree quietly.

use crate::from_b64url;

/// A browser's answer to `navigator.credentials.create()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registration {
    pub client_data_json: Vec<u8>,
    pub attestation_object: Vec<u8>,
}

/// A browser's answer to `navigator.credentials.get()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assertion {
    pub credential_id: Vec<u8>,
    pub client_data_json: Vec<u8>,
    pub authenticator_data: Vec<u8>,
    pub signature: Vec<u8>,
}

/// Why a page's body could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyError {
    NotJson,
    Field(&'static str),
}

impl core::fmt::Display for BodyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotJson => f.write_str("the body is not a JSON object"),
            Self::Field(name) => write!(
                f,
                "the body's `{name}` is missing, not a string, or not base64url without padding"
            ),
        }
    }
}

impl std::error::Error for BodyError {}

fn object(body: &[u8]) -> Result<serde_json::Map<String, serde_json::Value>, BodyError> {
    match serde_json::from_slice::<serde_json::Value>(body) {
        Ok(serde_json::Value::Object(map)) => Ok(map),
        _ => Err(BodyError::NotJson),
    }
}

fn bytes(
    map: &serde_json::Map<String, serde_json::Value>,
    name: &'static str,
) -> Result<Vec<u8>, BodyError> {
    map.get(name)
        .and_then(serde_json::Value::as_str)
        .and_then(from_b64url)
        .ok_or(BodyError::Field(name))
}

/// Read a registration answer: `clientDataJSON` and `attestationObject`.
///
/// # Errors
///
/// [`BodyError`] naming what is wrong.
pub fn registration(body: &[u8]) -> Result<Registration, BodyError> {
    let map = object(body)?;
    Ok(Registration {
        client_data_json: bytes(&map, "clientDataJSON")?,
        attestation_object: bytes(&map, "attestationObject")?,
    })
}

/// Read an assertion answer: `id`, `clientDataJSON`, `authenticatorData`
/// and `signature`.
///
/// # Errors
///
/// [`BodyError`] naming what is wrong.
pub fn assertion(body: &[u8]) -> Result<Assertion, BodyError> {
    let map = object(body)?;
    Ok(Assertion {
        credential_id: bytes(&map, "id")?,
        client_data_json: bytes(&map, "clientDataJSON")?,
        authenticator_data: bytes(&map, "authenticatorData")?,
        signature: bytes(&map, "signature")?,
    })
}

/// The one-time enrollment code a page sent, if it sent one: `{"code": "…"}`.
/// An empty body, or one without `code`, is `Ok(None)`.
///
/// # Errors
///
/// [`BodyError`] for a body that is not JSON, or a `code` that is not a string.
pub fn enrollment_code(body: &[u8]) -> Result<Option<String>, BodyError> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(None);
    }
    let map = object(body)?;
    match map.get("code") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(code)) => Ok(Some(code.clone())),
        Some(_) => Err(BodyError::Field("code")),
    }
}
