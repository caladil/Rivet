//! `rivet get trust` — **which signing keys this machine accepts.**
//!
//! `ADR-028` §5.5.6 is the ruling and it names this subject by name. It was
//! written on 2026-08-24, when the trust store moved out of the binaries and
//! into a file, and the reason it moved was not security:
//!
//! > Security is not the discriminator — both sit on the same unverified SD
//! > card, and §5.4 already concedes that an attacker who can write `/system`
//! > has already won. What decides it is that **a machine should be able to say
//! > what it trusts.** `rivet get trust` returning the labels, key ids and masks
//! > is a machine fact in `ADR-006` §3's sense; a constant in a stripped binary
//! > is a fact nobody can ask for.
//!
//! **Built 2026-08-29**, Pile B item 3. For five days the file existed and the
//! only things that could read it were the two binaries about to verify
//! something with it — so *"will this board install what I signed"* was
//! answerable only by signing something and trying. `crate::package::trust_store`
//! has said *"`rivet get trust` is how a human asks"* in its module
//! documentation since the day it was written, which made this the rare gap
//! that was documented as present before it existed.
//!
//! # The question this answers, and the field that answers it
//!
//! A `.rvt`'s header carries a `key_id` — `BLAKE3` of the public key that signed
//! it (`ADR-029` §3) — and `ADR-029` §5 step 6 turns verification into a table
//! lookup on that id. **So `key_id` is the field that joins an artifact to a
//! machine**, and it is what `cargo rivet inspect` prints for an artifact. This
//! subject prints it for a machine. Comparing the two is the whole workflow.
//!
//! The public key is reported beside it because the two answer different
//! questions: the id is what a *header* names, and the key is what an operator
//! has in the `.pub` file beside their seed.
//!
//! # An empty store is an ANSWER and an unreadable one is a refusal
//!
//! `ADR-028` §5.5.5 lets an image be built `--no-trust`, and such a machine
//! trusts nobody and can install nothing. That is a **reading**, and it is
//! reported as one: no keys, and a `means` that says what it costs.
//!
//! A store that is missing or will not parse is a **refusal**, in
//! `cmd/rivet-deploy`'s own words for the same file: *"a store that cannot be
//! read is not a store that trusts nobody, and treating it as one is how a
//! corrupt file becomes an install nobody authorised."* The refusal is the
//! adapter's, because the adapter is the thing that tried; this module reports
//! what it was told.
//!
//! # What it deliberately does not do
//!
//! **It does not verify anything, and it names no artifact.** Whether a
//! particular `.rvt` will install is a question about that file's header, its
//! signature, its kind entitlement, its arch and its platform — `ADR-029` §9.2's
//! whole sequence, on the machine. A subject that answered *"yes, this will
//! install"* from a trust store alone would be answering a question it had not
//! checked, and it would be believed.

use super::machine::Observed;
use super::{ErrorCode, Field, OperationError};
use crate::package::key_material;
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "trust";

/// What this subject's result carries. `ADR-006` §7.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was read. Always `trust` in this build.",
    },
    Field {
        name: "read_from",
        description: "Where the trust store was read, so the answer names a place. ADR-028 \
                      §5.5.6 fixes the path deliberately — a CONFIGURABLE trust anchor is the \
                      thing §5.2 exists to forbid — so a reader who finds anything else here \
                      has found a defect rather than a setting.",
    },
    Field {
        name: "keys",
        description: "How many keys this machine trusts, as a count rather than the length of a \
                      list a caller has to measure. ZERO IS A REAL ANSWER: an image built \
                      --no-trust has an empty store (ADR-028 §5.5.5) and can install nothing. It \
                      is not the same as an unreadable store, which is refused rather than \
                      reported as this.",
    },
    Field {
        name: "trusted",
        description: "Every entry, IN FILE ORDER and not de-duplicated — a store carrying one \
                      key twice is a fact about that file and folding the two would answer `one \
                      key` about a file holding two records. Each carries `id`, `key`, `label` \
                      and `may_sign`. `id` IS THE FIELD THAT JOINS AN ARTIFACT TO THIS MACHINE: \
                      it is BLAKE3 of the public key and it is what a payload header names \
                      (ADR-029 §3), so it is what `cargo rivet inspect` prints as `key`. `key` \
                      is the 32 raw bytes as hex, which is what an operator has in the `.pub` \
                      file beside their seed. `label` is a NAME FOR REFUSALS AND NEVER AN \
                      AUTHORITY — nothing is decided by comparing it (ADR-028 §5.5.1). \
                      `may_sign` is the list of <kind> tokens this key is entitled to sign, and \
                      an EMPTY list is a real state rather than a missing value: it is what a \
                      revoked entry becomes before the image carrying its removal is built \
                      (ADR-028 §5.5.4).",
    },
    Field {
        name: "means",
        description: "What this reading does and does not assert, in prose. The sentence it \
                      always contains is that NOTHING HERE VERIFIES ANYTHING: whether a \
                      particular artifact will install depends on its signature, its kind \
                      entitlement, its arch and its platform as well as its key, and that is \
                      ADR-029 §9.2's sequence on the machine rather than a question a trust \
                      store answers.",
    },
];

/// Perform it.
///
/// # Errors
///
/// If a second word was given, or the machine could not read its trust store.
pub(super) fn perform(
    arguments: &[&str],
    observed: &mut dyn Observed,
) -> Result<Value, OperationError> {
    if let Some(extra) = arguments.get(1) {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            SUBJECT,
            format!(
                "`{SUBJECT}` takes no second word and {extra:?} is one. The whole store is the \
                 answer; select one key from it with a JSON tool rather than with an argument \
                 this build does not have. And if {extra:?} was a path to an artifact, there is \
                 no such input: this subject reports what the machine trusts and verifies \
                 nothing (ADR-029 §9.2 is what decides whether a payload installs, on the \
                 machine). `cargo rivet inspect` is what reads an artifact."
            ),
        ));
    }

    let read = observed.read_trust().map_err(|e| {
        OperationError::about(ErrorCode::MachineRefused, SUBJECT, e.why().to_string())
    })?;

    let trusted: Vec<Value> = read
        .keys
        .iter()
        .map(|entry| {
            Value::object(vec![
                // The WHOLE id, not `KeyId`'s `Display`. That one prints eight
                // bytes and an ellipsis because a journal line has room for
                // eight bytes; this is a machine-readable answer whose whole
                // purpose is to be compared against a header's field, and a
                // truncated value that looks comparable is worse than none.
                ("id", Value::str(key_material::to_hex(&entry.id.0))),
                ("key", Value::str(key_material::to_hex(&entry.key.0))),
                ("label", Value::str(entry.label.clone())),
                (
                    "may_sign",
                    Value::List(
                        entry
                            .may_sign
                            .iter()
                            .map(|kind| Value::str(kind.as_str()))
                            .collect(),
                    ),
                ),
            ])
        })
        .collect();

    let fields = vec![
        ("subject", Value::str(SUBJECT)),
        ("read_from", Value::str(read.source.clone())),
        (
            "keys",
            Value::Integer(i64::try_from(read.keys.len()).unwrap_or(i64::MAX)),
        ),
        ("trusted", Value::List(trusted)),
        ("means", Value::str(means(&read))),
    ];

    debug_assert!(
        super::get::in_declared_order(&fields, OUTPUTS.iter().map(|f| f.name)),
        "a get trust result's keys must be a subsequence of OUTPUTS, in order"
    );
    Ok(Value::object(fields))
}

/// **What this reading asserts, and the two things it never does.**
///
/// Composed from what was read, for [`super::thermal::means`]'s reason: the
/// sentence that matters changes with the answer, and a reader handed the wrong
/// caveat is worse off than one handed none. The first paragraph is the same
/// every time, because it is true of every possible reading.
fn means(read: &super::machine::TrustRead) -> String {
    let mut said = String::from(
        "NOTHING HERE VERIFIES ANYTHING. This is the set of keys whose signatures this machine \
         will consider, and considering one is the first step of ADR-029 §9.2 rather than the \
         whole of it: a payload also has to have a signature that checks out, a kind this key is \
         entitled to sign, and an arch and a platform this machine accepts. A key being listed \
         here does not mean an artifact it signed will install. ",
    );

    if read.keys.is_empty() {
        said.push_str(
            "THIS MACHINE TRUSTS NOBODY. The store was read and holds no entries, which is what \
             an image built --no-trust looks like from here (ADR-028 §5.5.5) — so no update and \
             no application can be installed on it at all, and `apply install` will refuse \
             everything until an image update ships a store with a key in it. This is a reading \
             rather than a failure to take one: a store that could not be READ is refused by \
             this subject and never reported as an empty one.",
        );
        return said;
    }

    // Which kinds are covered at all, in the order they were first seen. An
    // operator's next question after "what does it trust" is almost always "can
    // it take an update", and the entitlement is the half that answers it —
    // `ADR-028` §5.5 exists because a key trusted for everything is not what
    // this system wants.
    let mut kinds: Vec<&str> = Vec::new();
    for entry in &read.keys {
        for kind in &entry.may_sign {
            if !kinds.contains(&kind.as_str()) {
                kinds.push(kind.as_str());
            }
        }
    }
    if kinds.is_empty() {
        said.push_str(&format!(
            "{} key(s) are listed and NOT ONE OF THEM MAY SIGN ANYTHING — every entitlement is \
             empty. That is the state a revoked entry is in before the image carrying its \
             removal is built (ADR-028 §5.5.4), and from an installer's point of view this \
             machine is in the same position as one with an empty store.",
            read.keys.len()
        ));
    } else {
        said.push_str(&format!(
            "{} key(s) are listed, between them entitled to sign: {}. A kind that is NOT in that \
             list cannot be installed on this machine by anybody, whatever they sign it with \
             (ADR-028 §5.5). Revocation is an A/B image update that ships a store without the \
             key (§5.5.4) — there is no expiry field and there is no command that removes one, \
             deliberately: an expired trust entry does not fail safe on a headless board, it \
             removes the ability to update the machine that needs updating.",
            read.keys.len(),
            kinds.join(", "),
        ));
    }
    said
}

#[cfg(test)]
#[path = "trust_tests.rs"]
mod tests;
