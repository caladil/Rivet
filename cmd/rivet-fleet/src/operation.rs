//! **The operation set, as data.** One table, two surfaces.
//!
//! `ADR-034` §10.1 is the ruling and the maintainer's reason is stronger than
//! the convenience one: the human and agentic interfaces must be genuinely
//! co-equal, which only one binary secures (.rulings/ R-053).
//!
//! > **One binary makes §4.1's rule structural instead of aspirational.** Two
//! > binaries would let the two surfaces drift and nothing would catch it; one
//! > binary cannot offer the human an operation the command line does not have,
//! > because there is only one set of them.
//!
//! One binary is necessary and not sufficient: two `match` statements inside
//! one binary drift exactly as two binaries would. So the operations are a
//! **table** — [`OPERATIONS`] — and both surfaces are built from it. The
//! `clap` command tree is generated from this list (`crate::cli`), and so is
//! the menu the terminal interface shows. Adding a row adds the operation to
//! both; there is no second place to add it to.
//!
//! # The vocabulary is `rivet`'s, and that is `ADR-034` §11.1
//!
//! The maintainer wants `rivet` and `rivet-fleet` to share one vocabulary
//! while the machinery under each differs, and judged adding Rivet's
//! vocabulary to Ignition the worse way to get there (.rulings/ R-055).
//!
//! Every [`Operation::verb`] below is a [`rivet_core::operation::Verb`] — the
//! same nine words `rivet` accepts, imported rather than re-spelled. A verb
//! this project does not have cannot be typed here, because the enum has no
//! variant for it. What differs is the machinery: `cmd/rivet` hand-tokenises
//! because it is a login shell and this uses `clap`.
//!
//! # What a row may not carry
//!
//! No function pointer. A table that dispatched would make this module the
//! composition root, and `ADR-014` gives that job to `main.rs`. The table
//! describes; `crate::run` decides. What that buys concretely is that the
//! terminal interface can render every operation — including ones it does not
//! yet know how to invoke — rather than silently showing a subset.

use rivet_core::operation::Verb;

/// One input an operation takes.
///
/// **Every input is a flag**, which is `ADR-034` §11.1's ruling and not a
/// stylistic preference: `rivet` puts its inputs in fixed positions because a
/// login shell's argument vector is tokenised by hand, and a fleet tool with
/// eight-field records would need eight remembered positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Input {
    /// The long flag, without the leading dashes. `--name`, never `-n`: a
    /// single letter is a second spelling to memorise, and `ADR-006` §6.4's
    /// argument against abbreviations applies to flags as well as to verbs.
    pub name: &'static str,
    /// Whether the operation refuses without it.
    pub required: bool,
    /// Whether it may be given more than once.
    pub several: bool,
    /// What it is, in a sentence a person reads in `--help`.
    pub description: &'static str,
}

/// One thing this tool can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Operation {
    pub verb: Verb,
    /// What the verb acts on — the second word. Singular or plural as the
    /// operation reads: `list devices`, `get device`.
    pub subject: &'static str,
    /// One line, shown by `--help` and by the terminal interface's menu.
    pub summary: &'static str,
    pub inputs: &'static [Input],
    /// Whether performing it changes something the operator would want warned
    /// about first.
    ///
    /// **`ADR-034` §4.2 — the human interface shows its work** — and the
    /// command line owes the same courtesy. An operation marked here prints
    /// what it would do and does nothing unless the operator confirms, which is
    /// `ADR-012` §7.1's mechanism reaching a host tool unchanged.
    pub confirms: bool,
}

/// The confirmation, spelled as `rivet` spells it.
///
/// **The literal word, as the last word**, and not `--yes`: an operator who has
/// typed `confirm` at a board has typed the thing that happens here too.
pub const CONFIRMATION: &str = "confirm";

/// Which wireless network a credential belongs to.
const NETWORK: Input = Input {
    name: "network",
    required: true,
    several: false,
    description: "the SSID. Not a secret, and it is what a device's record names.",
};

// **`--identity` and `--pin` are `crate::cli`'s globals rather than
// per-operation inputs**, because they describe the RUN rather than the
// operation: one operator identity reaches every machine a run touches
// (`ADR-034` §11.5.2).

/// Where a machine's control plane listens — `ADR-034` §6.1.
const LISTEN: Input = Input {
    name: "listen",
    required: true,
    several: false,
    description: "the address and port rivet-mcpd binds, as `0.0.0.0:7777`. THE REGISTER AND \
                  THE DOCUMENT ARE WRITTEN TOGETHER (ADR-034 6.1) - both, or neither - \
                  because this value is how this tool reaches the machine to change it.",
};

/// Which service an operation acts on.
///
/// **A free string, not a closed list**, and that is deliberate. The services
/// a machine can run are the image's (`cmd/rivet-sysd`'s definitions) plus
/// every application installed on it (`ADR-015`), and the register does not
/// know either — it composes a document and the machine refuses a name it does
/// not have. A list here would be a second, staler answer to *what can this
/// machine run* (`ADR-011` §3).
/// A cap on a service's memory, optional everywhere it appears.
///
/// **Optional, and its absence is not `unlimited`** — it means *say nothing*,
/// which leaves whatever the record already held. `none` is how a cap is
/// removed, because an operator who wanted to clear one and typed nothing
/// would otherwise be told the register changed when it had not.
///
/// **Checked here rather than on the machine.** `rivet_core::service::MemoryLimit`
/// parses it at the moment it is recorded, so a document this register composes
/// cannot carry a value the machine would refuse — which is `ADR-034`'s whole
/// claim about the register being the authority.
const MEMORY: Input = Input {
    name: "memory",
    required: false,
    several: false,
    description: "a cap on the service's memory, in ADR-007 \u{a7}7.2's spelling - \
                  `32M`, `512K`, `1G`. The literal word `none` removes a cap this \
                  record already holds; omitted entirely, the record's cap is left \
                  alone. It is checked HERE, so a composed document cannot carry a \
                  value the machine would refuse.",
};

/// A control-plane public key, as hex.
///
/// **A whole line of key material is refused and the refusal says so**, because
/// the two are pasted from different places and confusing them produces a
/// machine that looks configured and refuses every caller. `ADR-025` §2.2 keeps
/// the channels separate on purpose.
const PEER_KEY: Input = Input {
    name: "key",
    required: true,
    several: false,
    description: "an Ed25519 public key as 64 hex characters - what `cargo rivet identity \
                  new` prints, or what `rivet-fleet add identity` recorded. NOT a whole line \
                  of key material: this is ADR-025 \u{a7}2's control-plane identity, and it is \
                  its own channel with its own lifetime.",
};

const SERVICE: Input = Input {
    name: "service",
    required: true,
    several: false,
    description: "the service, by the name the machine knows it by - `rivet-mdnsd`, \
                  `rivet-timed`, or an installed application's own name.",
};

/// A carried product, in `[carry.<product>]` — `.rulings/` R-159 R20, R-160
/// R26.
///
/// **The same grammar `SERVICE` uses**, because `rivet_config::carry`'s
/// header makes the two tables share one vocabulary for what a product name
/// is (`ADR-011`) - a pin names the same product a service entry enables.
const PRODUCT: Input = Input {
    name: "product",
    required: true,
    several: false,
    description: "the product, by the name a fleet repository publishes it under - the same \
                  grammar a service name uses.",
};

/// The version a carry pin names.
///
/// **`none` clears a pin** — `.rulings/` R-162 R30, `MEMORY`'s footing
/// exactly: the literal word removes whatever this record already holds, and
/// leaving the flag out entirely leaves the record alone. Required, unlike
/// `MEMORY`, because `set carry`/`set fleet-carry` have no OTHER effect to
/// perform with the flag omitted — there is no `enabled` beside it the way
/// `set service` has.
const CARRY_VERSION: Input = Input {
    name: "version",
    required: true,
    several: false,
    description: "which published version this role's card must carry, exactly as `cargo \
                  rivet fleet publish` recorded it. The literal word `none` clears a pin this \
                  record already holds. ADR-050 \u{a7}5.2: a card build refuses rather than \
                  guesses when the repository holds more than one version and nothing pins one.",
};

/// Which device an operation acts on.
///
/// One definition, because every device-shaped operation takes exactly this and
/// six copies of a sentence is six chances to describe it differently.
const DEVICE: Input = Input {
    name: "device",
    required: true,
    several: false,
    description: "which device, by its human label or by its opaque identifier. ADR-025 \u{a7}4.1 \
                  makes the identifier authoritative and the label a mutable alias, so both \
                  resolve and an ambiguous label is refused rather than guessed at.",
};

/// **Every operation this tool has.** Ordered by verb, in `ADR-006` §6.4's
/// order, then by subject.
///
/// A row here is an operation both surfaces have. There is no second list.
/// The repository an operation names.
///
/// **By name, never by identifier.** The identifier is authoritative for what a
/// repository IS (`ADR-025` §4.1) and is deliberately not what an operator
/// types: it is minted, opaque, and sixteen characters somebody would have to
/// copy. `rivet-fleet` is the thing that holds the mapping, which is the whole
/// reason it holds it.
const REPO: Input = Input {
    name: "repo",
    required: true,
    several: false,
    description: "the repository, by name. Names are unique per device, which is what makes \
                  this enough.",
};

pub const OPERATIONS: &[Operation] = &[
    Operation {
        verb: Verb::Get,
        subject: "device",
        summary: "Everything the register records about one device.",
        inputs: &[DEVICE],
        confirms: false,
    },
    Operation {
        verb: Verb::Get,
        subject: "status",
        summary: "Ask a member what it is running, in its own words (ADR-032 3.3).",
        inputs: &[DEVICE],
        confirms: false,
    },
    Operation {
        verb: Verb::Set,
        subject: "listen",
        summary: "Move where this machine's control plane listens - REGISTER AND MACHINE \
                  TOGETHER (ADR-034 6.1).",
        inputs: &[DEVICE, LISTEN],
        confirms: true,
    },
    Operation {
        verb: Verb::Set,
        subject: "pin",
        summary: "Record this machine's control-plane public key, so commands stop needing \
                  --pin (ADR-034 6.1).",
        inputs: &[
            DEVICE,
            Input {
                name: "pin",
                required: true,
                several: false,
                description: "the MACHINE's Ed25519 public key, 64 hex characters - what \
                              `cargo rivet identity new` printed when its card was built. \
                              RECORDS a key the machine already has; it mints nothing and \
                              sends nothing. `add identity` is the one that mints, and that \
                              needs a new card.",
            },
        ],
        confirms: false,
    },
    Operation {
        verb: Verb::Set,
        subject: "fleet-role",
        summary: "Record which SPEC-05 5 role this device's document should default from \
                  (ADR-004 6).",
        inputs: &[
            DEVICE,
            Input {
                name: "role",
                required: true,
                several: false,
                description: "`server` or `member` - rivet-provision's own vocabulary for the \
                              same question, so the two tools apply the same defaults for the \
                              same word. NOT the free-text `--role` `add device` takes - that \
                              is a label for a human reading the list, and this is a \
                              controlled default this record composes from.",
            },
        ],
        confirms: false,
    },
    Operation {
        verb: Verb::Set,
        subject: "timezone",
        summary: "Record this device's timezone directly, for a record with no machine to \
                  adopt one from yet (SPEC-05 5).",
        inputs: &[
            DEVICE,
            Input {
                name: "timezone",
                required: true,
                several: false,
                description: "an IANA time zone identifier - `America/Chicago`, \
                              `Europe/Berlin`, `UTC`. Checked for GRAMMAR only, the same \
                              check rivet-config's own parser applies; WHETHER THE ZONE \
                              EXISTS is not checked here (SPEC-05 5) - a well-formed name \
                              for a zone no image carries is reported by the machine's own \
                              projection instead, at first boot.",
            },
        ],
        confirms: false,
    },
    Operation {
        verb: Verb::Set,
        subject: "kiosk-auto-run",
        summary: "Record which application takes this device's display (SPEC-05 5).",
        inputs: &[
            DEVICE,
            Input {
                name: "application",
                required: true,
                several: false,
                description: "the application the launcher should auto-run. NOTHING HERE \
                              CAN CHECK IT WILL BE INSTALLED - SPEC-05 5 names that gap on \
                              purpose - so a name matching nothing takes the display and \
                              draws nothing on it, silently.",
            },
        ],
        confirms: false,
    },
    Operation {
        verb: Verb::Set,
        subject: "fleet",
        summary: "Change a value every machine in this fleet shares (ADR-007 \u{a7}3.4 layer 3).",
        inputs: &[
            Input {
                name: "name",
                required: false,
                several: false,
                description: "a label for this fleet, for a human looking at two registers.",
            },
        ],
        confirms: false,
    },
    Operation {
        verb: Verb::Set,
        subject: "service",
        summary: "Turn a service on or off for one device (ADR-034 \u{a7}5).",
        inputs: &[
            DEVICE,
            SERVICE,
            Input {
                name: "enabled",
                required: true,
                several: false,
                description: "`true` or `false`. This is the ENTIRE control surface for a \
                              service (ADR-017 \u{a7}6) - there is no start, no stop and no \
                              restart, because a machine that could be told to start something \
                              its own configuration says is off would have two answers.",
            },
            MEMORY,
        ],
        confirms: false,
    },
    Operation {
        verb: Verb::Set,
        subject: "fleet-service",
        summary: "Turn a service on or off for every device that does not say otherwise.",
        inputs: &[
            SERVICE,
            Input {
                name: "enabled",
                required: true,
                several: false,
                description: "`true` or `false`, applied as ADR-007 \u{a7}3.4's layer 3 - a \
                              device's own answer still wins, per service rather than per table.",
            },
            MEMORY,
        ],
        confirms: false,
    },
    Operation {
        verb: Verb::Set,
        subject: "carry",
        summary: "Pin which published version of a product one device's card must carry.",
        inputs: &[DEVICE, PRODUCT, CARRY_VERSION],
        confirms: false,
    },
    Operation {
        verb: Verb::Set,
        subject: "fleet-carry",
        summary: "Pin which published version of a product every device carries, unless its \
                  own record says otherwise.",
        inputs: &[
            PRODUCT,
            Input {
                name: "version",
                required: true,
                several: false,
                description: "which published version, applied as ADR-007 \u{a7}3.4's layer 3 \
                              - a device's own pin still wins, per product rather than per \
                              table (ADR-050 \u{a7}5.2 - no role knows a version, so this is \
                              the only layer beneath a device's own). The literal word `none` \
                              clears the fleet-wide pin this record already holds - \
                              `.rulings/` R-162 R30.",
            },
        ],
        confirms: false,
    },
    Operation {
        verb: Verb::Set,
        subject: "repo",
        summary: "Rename a repository. The machine MOVES it; nothing is created and nothing lost.",
        inputs: &[
            DEVICE,
            REPO,
            Input {
                name: "name",
                required: true,
                several: false,
                description: "the new name. The identifier does not change, which is what \
                              makes this a rename rather than a delete and a create \
                              (ADR-025 \u{a7}4.1) - the machine moves the directory and every \
                              commit in it comes along.",
            },
        ],
        confirms: false,
    },
    Operation {
        verb: Verb::List,
        subject: "services",
        summary: "What each device runs, and whether that came from the device or the fleet.",
        inputs: &[],
        confirms: false,
    },
    Operation {
        verb: Verb::List,
        subject: "carry",
        summary: "Which version of each carried product every device's card will build with, \
                  and whether that came from the device or the fleet - `.rulings/` R-162 R30.",
        inputs: &[],
        confirms: false,
    },
    Operation {
        verb: Verb::List,
        subject: "devices",
        summary: "Every device in the register, by label, role and location.",
        inputs: &[],
        confirms: false,
    },
    Operation {
        verb: Verb::List,
        subject: "repos",
        summary: "Every repository this register declares, by device, with its identifier.",
        inputs: &[],
        confirms: false,
    },
    Operation {
        verb: Verb::List,
        subject: "credentials",
        summary: "Which wireless networks this operator holds a key for. Networks only, never keys.",
        inputs: &[],
        confirms: false,
    },
    Operation {
        verb: Verb::Add,
        subject: "device",
        summary: "Enroll a device: mint its opaque identifier and write its record.",
        inputs: &[
            Input {
                name: "name",
                required: true,
                several: false,
                description: "the human label. Mutable, and not what the record is filed under.",
            },
            Input {
                name: "role",
                required: false,
                several: false,
                description: "what this machine is for, in one word.",
            },
            Input {
                name: "location",
                required: false,
                several: false,
                description: "where it physically is, for a human reading the list.",
            },
            Input {
                name: "fleet-role",
                required: false,
                several: false,
                description: "`server` or `member` - SPEC-05 5's role, which this device's \
                              document composes defaults from (ADR-004 6). NOT --role: that is \
                              a free label for a human reading the list. Omitted, no role \
                              default applies; `set fleet-role` answers it later.",
            },
            // **Read once, on the FIRST enrollment, and ignored afterwards.**
            // There is no `init` verb, so the first `add device` is the only
            // moment guaranteed to happen exactly once per fleet. It is here
            // rather than on `set fleet` so that a scripted first run can name
            // the fleet without a second command.
            Input {
                name: "fleet",
                required: false,
                several: false,
                description: "a label for this FLEET, asked for once if the register has none. \
                              Not the certificate authority's fleet label - that one is frozen \
                              into a root certificate and this one is not.",
            },
        ],
        confirms: false,
    },
    // **`add document` is `apply document` read backwards**, and the pair is
    // deliberate: `apply` pushes the register's document at the machine, `add`
    // pulls what the machine holds back into the register. `ADR-006` §6.4's
    // vocabulary has no *adopt*, and inventing a tenth verb for one operation
    // would be a worse trade than using the one whose gloss already says *put
    // something into the register* — which is exactly what this does.
    Operation {
        verb: Verb::Add,
        subject: "document",
        summary: "Record what this MACHINE's document holds and the register does not - the \
                  adoption ADR-034 \u{a7}6.2 recommends, and what clears its refusal.",
        inputs: &[DEVICE],
        confirms: true,
    },
    Operation {
        verb: Verb::Add,
        subject: "repo",
        summary: "Declare a git repository on one device: mint its identifier and record it.",
        inputs: &[
            DEVICE,
            Input {
                name: "name",
                required: true,
                several: false,
                description: "what the repository is called. It becomes the directory on the \
                              machine and the path a client asks for.",
            },
        ],
        confirms: false,
    },
    Operation {
        verb: Verb::Add,
        subject: "credential",
        summary: "Record the pairwise master key for a wireless network (ADR-034 \u{a7}7).",
        inputs: &[
            NETWORK,
            Input {
                name: "key",
                required: false,
                several: false,
                description: "the PMK, as 64 hexadecimal characters - the `psk=` value \
                              `wpa_passphrase` prints. NOT the passphrase: ADR-004 \u{a7}5 \
                              rules that the plaintext passphrase is never written to \
                              persistent storage, and this tool refuses one rather than \
                              storing it.",
            },
            Input {
                name: "passphrase",
                required: false,
                several: false,
                description: "the network's passphrase. THIS BUILD REFUSES IT and says why: \
                              WPA2's derivation is PBKDF2-HMAC-SHA1 and this workspace has \
                              neither primitive. The flag exists so that somebody holding a \
                              passphrase is told that, rather than finding no flag and \
                              guessing at what to do.",
            },
        ],
        confirms: false,
    },
    Operation {
        verb: Verb::Add,
        subject: "peer",
        summary: "Record a public key allowed to reach a control plane (ADR-025 \u{a7}2).",
        inputs: &[
            PEER_KEY,
            Input {
                name: "device",
                required: false,
                several: false,
                description: "the device this key may reach. OMITTED, the key is the FLEET's \
                              and every machine accepts it - which is ADR-025 \u{a7}2's \
                              \"the device holds the fleet's public key\" and is the usual \
                              case for an operator's own workstation.",
            },
        ],
        confirms: false,
    },
    Operation {
        verb: Verb::Add,
        subject: "identity",
        summary: "Mint this device's TLS identity, write it for a card build, and keep only \
                  the public half (ADR-025 \u{a7}3).",
        inputs: &[
            DEVICE,
            Input {
                name: "out",
                required: true,
                several: false,
                description: "a directory to write device.seed, device.crt and peers into - \
                              what tools/card/build.sh takes as RIVET_IDENTITY. THE SEED IS A \
                              PRIVATE KEY and this is the only moment it exists outside a \
                              card: ADR-034 \u{a7}3.5 keeps it out of the register entirely, \
                              and ADR-019 \u{a7}3.1 has the board erase it from the card on \
                              first boot.",
            },
        ],
        confirms: true,
    },
    Operation {
        verb: Verb::Remove,
        subject: "repo",
        summary: "Stop declaring a repository. THE DATA IS NOT DELETED - `apply repo-delete` \
                  is what destroys it, and it requires this first.",
        inputs: &[DEVICE, REPO],
        confirms: true,
    },
    Operation {
        verb: Verb::Remove,
        subject: "credential",
        summary: "Forget one network's key. Machines already provisioned with it keep working.",
        inputs: &[NETWORK],
        confirms: true,
    },
    Operation {
        verb: Verb::Remove,
        subject: "device",
        summary: "Forget a device. The register stops recording it; nothing on the machine changes.",
        inputs: &[DEVICE],
        confirms: true,
    },
    Operation {
        verb: Verb::Remove,
        subject: "peer",
        summary: "Withdraw a control-plane key. ADR-025 \u{a7}5: revocation is removing it \
                  from a list.",
        inputs: &[
            PEER_KEY,
            Input {
                name: "device",
                required: false,
                several: false,
                description: "the device to take it from. Omitted, it is taken from the fleet.",
            },
        ],
        confirms: true,
    },
    Operation {
        verb: Verb::Apply,
        subject: "document",
        summary: "Compose this device's document onto the machine and make it re-read it.",
        inputs: &[DEVICE],
        confirms: true,
    },
    Operation {
        verb: Verb::Apply,
        subject: "repo-delete",
        summary: "DESTROY a repository on a device, and every commit in it. Not recoverable.",
        inputs: &[
            DEVICE,
            Input {
                name: "repo",
                required: true,
                several: false,
                description: "the repository to destroy, by name.",
            },
            Input {
                name: "id",
                required: true,
                several: false,
                description: "the identifier it should have. THE MACHINE COMPARES IT to the \
                              one in the directory and destroys nothing if they differ - a \
                              name is a label and labels get reused (ADR-025 \u{a7}4.1). \
                              Typed rather than looked up, because this is the one command \
                              where being wrong is permanent.",
            },
        ],
        confirms: true,
    },
    Operation {
        verb: Verb::Apply,
        subject: "trial-restart",
        summary: "Restart the machine ONCE into the other A/B slot (ADR-018 5).",
        inputs: &[DEVICE],
        confirms: true,
    },
    Operation {
        verb: Verb::Export,
        subject: "provisioning",
        summary: "Compose this device's provisioning document and print it (ADR-004 \u{a7}6).",
        inputs: &[DEVICE],
        confirms: false,
    },
    Operation {
        verb: Verb::Export,
        subject: "card-plan",
        summary: "The steps to provision and write a card for this device, with its own \
                  values already in them.",
        inputs: &[
            DEVICE,
            Input {
                name: "card-mib",
                required: false,
                several: false,
                description: "the size of the PHYSICAL card, in MiB. Optional, and the plan \
                              says STOP without it rather than guessing: /state is sized at \
                              build time from this number and nothing on the running machine \
                              grows it, so a card larger than the image leaves a /state that \
                              cannot be fixed remotely (PLAN.md 13.4).",
            },
        ],
        confirms: false,
    },
    Operation {
        verb: Verb::Explain,
        subject: "device",
        summary: "Where each value in this device's composed document came from.",
        inputs: &[DEVICE],
        confirms: false,
    },
];

/// The operation spelled `<verb> <subject>`, or `None`.
///
/// **The only lookup**, so that a surface cannot reach an operation the table
/// does not list. `cmd/rivet` learned this the hard way on 2026-08-26, when
/// `apply reload` shipped reachable and uncatalogued because dispatch and the
/// catalogue were two lists.
#[must_use]
pub fn find(verb: Verb, subject: &str) -> Option<&'static Operation> {
    OPERATIONS
        .iter()
        .find(|op| op.verb == verb && op.subject == subject)
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **The vocabulary is `rivet`'s.** Asserted rather than trusted, because
    /// the failure `ADR-034` §11.1 is about is a fleet tool that quietly grows
    /// a tenth verb and stops being predictable from the machine tool.
    #[test]
    fn every_verb_is_one_rivet_already_speaks() {
        for op in OPERATIONS {
            assert!(
                Verb::ALL.contains(&op.verb),
                "{} is not one of ADR-006 6.4's verbs",
                op.verb.as_str()
            );
        }
    }

    /// A verb and a subject together are the identity of an operation, so two
    /// rows sharing both are two answers to one command.
    #[test]
    fn no_two_operations_are_spelled_the_same_way() {
        let mut seen: Vec<(&str, &str)> = Vec::new();
        for op in OPERATIONS {
            let key = (op.verb.as_str(), op.subject);
            assert!(!seen.contains(&key), "{key:?} appears twice");
            seen.push(key);
        }
    }

    /// The table is ordered by `ADR-006` §6.4's verb order, so `--help` reads
    /// in the order the vocabulary is documented in rather than in the order
    /// rows happened to be added.
    #[test]
    fn the_table_is_in_the_verb_order_the_adr_publishes() {
        let position = |verb: Verb| {
            Verb::ALL
                .iter()
                .position(|v| *v == verb)
                .expect("a verb not in Verb::ALL")
        };
        let mut last = 0;
        for op in OPERATIONS {
            let here = position(op.verb);
            assert!(here >= last, "{} comes after a later verb", op.verb.as_str());
            last = here;
        }
    }

    /// A flag with no description is one `--help` cannot explain, and an
    /// operation with no summary is one the terminal interface would show as a
    /// blank line.
    #[test]
    fn every_operation_and_every_input_says_what_it_is() {
        for op in OPERATIONS {
            assert!(!op.summary.is_empty(), "{} has no summary", op.subject);
            assert!(
                op.summary.ends_with('.'),
                "{}'s summary is not a sentence: {:?}",
                op.subject,
                op.summary
            );
            for input in op.inputs {
                assert!(
                    !input.description.is_empty(),
                    "{} --{} has no description",
                    op.subject,
                    input.name
                );
                assert!(
                    input.name.len() > 1,
                    "--{} is a single letter, and ADR-006 6.4 refuses abbreviations",
                    input.name
                );
            }
        }
    }

    /// Two inputs with one name on one operation is a flag whose value depends
    /// on which row `clap` reached first.
    #[test]
    fn an_operations_inputs_are_all_differently_named() {
        for op in OPERATIONS {
            let mut seen: Vec<&str> = Vec::new();
            for input in op.inputs {
                assert!(
                    !seen.contains(&input.name),
                    "{} declares --{} twice",
                    op.subject,
                    input.name
                );
                seen.push(input.name);
            }
        }
    }

    /// The lookup is the table's, so an operation that cannot be found cannot
    /// be performed.
    #[test]
    fn every_row_is_findable_and_nothing_else_is() {
        for op in OPERATIONS {
            assert_eq!(find(op.verb, op.subject), Some(op));
        }
        assert_eq!(find(Verb::Get, "frobnicator"), None);
        assert_eq!(find(Verb::Set, "device"), None);
    }
}
