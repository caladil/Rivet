//! **A ceremony captured from a real browser**, as bytes.
//!
//! PROVENANCE: captured from a real browser — Chrome on Windows, the credential
//! held by Windows Hello (`attachment: platform`), against `passkey-probe` on
//! dev-01 at `https://dev-01.local/passkey/`, which journals every byte a
//! relying party verifies. Unlike the W3C vectors and the self-generated ones,
//! a real authenticator wrote these, so agreeing with them is evidence that
//! this crate verifies what browsers actually send.
//!
//! The constants live apart from `captured_tests` so that `rivet-proxyd`'s
//! tests can drive the same ceremony through the relying party's state — the
//! store, the challenge table and the name a request arrived at — by compiling
//! this file in by path.

#![allow(dead_code)]

/// The RP ID and origin the ceremony ran at.
pub const RP_ID: &str = "dev-01.local";
pub const ORIGIN: &str = "https://dev-01.local";

pub const CREATE_CHALLENGE: &str = "MR2DMaKb7fdPwI3RH6JvdWWxlGCx9QLSCQJiEnISGRY";
pub const CREATE_CLIENT_DATA: &str = "eyJ0eXBlIjoid2ViYXV0aG4uY3JlYXRlIiwiY2hhbGxlbmdlIjoiTVIyRE1hS2I3ZmRQd0kzUkg2SnZkV1d4bEdDeDlRTFNDUUppRW5JU0dSWSIsIm9yaWdpbiI6Imh0dHBzOi8vZGV2LTAxLmxvY2FsIiwiY3Jvc3NPcmlnaW4iOmZhbHNlfQ";
pub const CREATE_ATTESTATION: &str = "o2NmbXRkbm9uZWdhdHRTdG10oGhhdXRoRGF0YViUacihnAeu94BzAuH_b0txHljjXmUagbOvIc6CgPtk8R1dAAAAAOqbjWZNAR0hPOS2tIy1ddQAEGboTL1IYtOikd9GEImVEZ2lAQIDJiABIVggvg6Um_N-neSTwDBls7wZjQ2DnJxXZ30tKLJdW096wZwiWCDSvrDRIxvZjVsaFDq_gWCSU3JxE3I_7dihjbsDB111SA";

pub const CREDENTIAL_ID: &str = "ZuhMvUhi06KR30YQiZURnQ";
pub const GET_CHALLENGE: &str = "VF4HXci2lsWjCkcEJoG0reraPRLjUQQq2kxBfvz4948";
pub const GET_CLIENT_DATA: &str = "eyJ0eXBlIjoid2ViYXV0aG4uZ2V0IiwiY2hhbGxlbmdlIjoiVkY0SFhjaTJsc1dqQ2tjRUpvRzByZXJhUFJMalVRUXEya3hCZnZ6NDk0OCIsIm9yaWdpbiI6Imh0dHBzOi8vZGV2LTAxLmxvY2FsIiwiY3Jvc3NPcmlnaW4iOmZhbHNlLCJvdGhlcl9rZXlzX2Nhbl9iZV9hZGRlZF9oZXJlIjoiZG8gbm90IGNvbXBhcmUgY2xpZW50RGF0YUpTT04gYWdhaW5zdCBhIHRlbXBsYXRlLiBTZWUgaHR0cHM6Ly9nb28uZ2wveWFiUGV4In0";
pub const GET_AUTH_DATA: &str = "acihnAeu94BzAuH_b0txHljjXmUagbOvIc6CgPtk8R0dAAAAAA";
pub const GET_SIGNATURE: &str = "MEUCID3C_RioK0F-CNMp51Bfk0rPvIeM8BoP0dHJ86A-nHq2AiEAgkxL3P3zw8atMpIAGRmfWrf-bA9zJJYrU0Q7sa3jFJo";
