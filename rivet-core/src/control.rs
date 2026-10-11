//! **What to do instead**, for a refusal that has just told somebody this
//! build cannot answer their question.
//!
//! # Why one constant rather than a sentence per refusal
//!
//! An operator reads a refusal at the moment they are already stuck, and
//! *"not this"* with no *"but this"* is where a person invents their own
//! answer. Six refusals in `cmd/rivet` once carried six hand-written versions
//! of this sentence, and when the answer changed, five of them kept the old
//! one. `ADR-011`'s one-authority rule applied to the text an operator reads
//! precisely when they need it to be right.

/// The one way to reach a running machine, as a sentence a refusal can append.
///
/// `ADR-035` is the control plane: MCP over TLS on port 7777, mutually
/// authenticated with pinned Ed25519 keys. `cargo rivet run` is the client.
pub const ASK_A_BOARD: &str = "Ask a board instead: `cargo rivet run --host <host>:7777 \
                               --identity <dir> --pin <hex> -- <command>` (ADR-035). There is \
                               no unauthenticated path.";

#[cfg(test)]
mod tests {
    use super::*;

    /// The sentence has to name the client, the port and the two credentials,
    /// because an operator who has just been refused has nothing else to go on.
    #[test]
    fn the_answer_is_complete_enough_to_act_on() {
        for part in ["cargo rivet run", "--host", "7777", "--identity", "--pin"] {
            assert!(ASK_A_BOARD.contains(part), "{part}");
        }
    }
}
