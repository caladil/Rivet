//! **What the card carries for this machine's applications** — `ADR-049`, the
//! producing end.
//!
//! `rivet_core::staging` is the format and `cmd/rivet-image` is the carrier;
//! this is where a *role*'s needs are decided, because composing them wants the
//! authority, the hostname and the machine's identity in one process at one
//! moment, and that is what this module is handed.
//!
//! # The list, and where it came from
//!
//! Not invented here. `SPEC-05` §10 is a manifest read back off `rivet-01` with
//! `get file` on 2026-09-07, after a working fleet server had been reached by
//! hand — three secret installations, two directory creations, three file
//! writes, five ownership changes and two service restarts. Every line below is
//! one of those.
//!
//! For a **fleet server**:
//!
//! | file | who reads it |
//! | :--- | :--- |
//! | `server.crt`, `server.key`, `ca.crt` | `rivet-fleetd`, for the browser |
//! | `operator-device.seed.hex`, `operator-device.crt.hex` | `rivet-fleetd`, to ask machines — **hex text, not bytes** |
//! | `register/register.toml`, `register/devices/<id>.toml` | `rivet-fleetd`, so it knows its own fleet |
//! | `tls-cert`, `tls-key`, `tls-root` | `rivet-proxyd`, the machine's front door |
//!
//! **Three naming schemes in one directory**, and that is not tidiable from
//! here: `cmd/rivet-fleetd` reads `server.crt` and `cmd/rivet-proxyd` reads
//! `tls-cert`, and each program is the authority on the names it opens. This
//! module writes what they read.
//!
//! # A member's front door is staged exactly as a server's
//!
//! Ruled (`.rulings/` R-160 R25). A fleet member enables `rivet-proxyd` too
//! (`ADR-050` §5), so it needs the same three files — its **own** leaf
//! certificate and key, minted from the fleet CA for its own name, plus the
//! same public root every machine's front door checks operator certificates
//! against. It goes through the same `crate::ca::issue_from` call a server
//! uses, just for the member's own name rather than the server's. **The
//! `rivet-fleetd` files stay server-only** — `server.crt`, `server.key`,
//! `ca.crt`, the operator hex pair and `register/` all describe the one
//! machine that runs the fleet register, and a member runs none of that. A
//! member's own key never leaves it; only the public root is shared, which is
//! what lets one operator certificate open every machine's pages (`ADR-046`).
//! A stand-alone machine's case is unchanged by this — see below.
//!
//! # Why the register is CARRIED and not created on the machine
//!
//! `cmd/rivet-fleetd`'s own words: it *"does not invent entries. A fleet server
//! that met an empty directory and minted a record for itself would be
//! manufacturing the one thing `ADR-034` says it cannot have: a device record
//! is written by the tool that GENERATED the device's keypair, and this program
//! generated nobody's."* So the fleet server's case is not *come up and create a
//! register* — it is *come up carrying a one-record register the tool that
//! minted your identity wrote*. This is that tool and this is that write.
//!
//! # Why a stand-alone machine carries none
//!
//! Ruled by the maintainer, 2026-09-07: a machine drops what it does not need
//! (.rulings/ R-303). The workstation holds the register for a machine it administers directly, and a
//! second copy on the machine would be the second authority `ADR-034` dropped
//! Local mode to avoid.

use std::path::{Path, PathBuf};

use rivet_core::staging::Staged;

/// The application that serves the fleet.
const FLEETD: &str = "rivet-fleetd";
/// The application that is the machine's front door.
const PROXYD: &str = "rivet-proxyd";

/// `rivet-fleetd`'s three, from `cmd/rivet-fleetd/src/main.rs`.
const SERVER_CERTIFICATE: &str = "server.crt";
const SERVER_KEY: &str = "server.key";
const AUTHORITY: &str = "ca.crt";

/// Its other two, from `cmd/rivet-fleetd/src/node.rs`. **Hex text**, which is
/// what that program reads and what every machine already carrying them holds.
const OPERATOR_SEED: &str = "operator-device.seed.hex";
const OPERATOR_CERTIFICATE: &str = "operator-device.crt.hex";

/// `rivet-proxyd`'s three, from `cmd/rivet-proxyd/src/main.rs`.
const TLS_CERT: &str = "tls-cert";
const TLS_KEY: &str = "tls-key";
const TLS_ROOT: &str = "tls-root";

/// Where the register goes inside `rivet-fleetd`'s data directory, from
/// `cmd/rivet-fleetd/src/fleet.rs`.
const REGISTER_DIR: &str = "register";

/// What was composed, and what has to be said about it.
pub struct Plan {
    /// Every file the card will carry.
    pub items: Vec<Staged>,
    /// Lines for the operator, in order. **Every one of them is a fact about
    /// what was written**, not advice.
    pub notes: Vec<String>,
    /// This machine's own leaf — already staged onto the card as part of
    /// `items` above — and the fleet name (`ADR-…`'s subject, `names[0]`) to
    /// write it under on the WORKSTATION. `None` only for a stand-alone
    /// machine, which mints no leaf at all.
    ///
    /// **Added for Fix 2.** `compose` mints and composes only — the doc
    /// comment on it says nothing is written until everything is composed —
    /// so the actual workstation write happens in the caller, through
    /// `crate::write_server_leaf`, once `items` itself has been written and
    /// read back successfully. That is what leaves
    /// `~/.rivet/ca/server-<fleet name>.crt`/`.key` for `cargo rivet fleet`'s
    /// "stage material (fleet-tls)" step to read on a later deploy, without
    /// making a provisioning run that fails partway through leave a stray
    /// workstation certificate behind.
    pub workstation_leaf: Option<(String, crate::ca::Minted)>,
}

impl core::fmt::Debug for Plan {
    /// **Hand-written, not derived** — `crate::ca::Minted` deliberately does
    /// not implement `Debug` (it holds a private key, and a panic message is
    /// a place a key must never appear), so a derive on `Plan` would not
    /// compile once `workstation_leaf` carries one. Only the subject name is
    /// printed; the leaf itself is not.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Plan")
            .field("items", &self.items)
            .field("notes", &self.notes)
            .field(
                "workstation_leaf",
                &self.workstation_leaf.as_ref().map(|(subject, _)| subject),
            )
            .finish()
    }
}

impl Plan {
    fn empty() -> Self {
        Self {
            items: Vec::new(),
            notes: Vec::new(),
            workstation_leaf: None,
        }
    }
}

/// The answers this needs, gathered so the caller passes one thing.
pub struct Machine<'a> {
    /// `server` or `member` — the only two words the interview offers since
    /// `ADR-059` §8, 2026-09-22. `"stand-alone"` still arrives here from an
    /// old saved answers file (the interview never writes it fresh any more)
    /// and is handled identically to `server`/`member` not being the answer —
    /// see [`compose`]'s own opening check.
    pub fleet: &'a str,
    /// The machine's name, one label.
    pub hostname: &'a str,
    /// The `[control-plane] listen` value the document will carry.
    pub listen: &'a str,
    /// `dhcp` or `static`.
    pub method: &'a str,
    /// The address, under `static`.
    pub address: &'a str,
    /// The gateway, under `static`.
    pub gateway: &'a str,
    /// `~/.rivet/ca`.
    pub authority: &'a Path,
    /// The directory holding this card's `device.seed`, `device.crt` and
    /// `peers` — what `cargo rivet provision` puts on the card. **`device.seed` is never
    /// opened**; the public key is in the certificate.
    pub identity: Option<&'a Path>,
    /// The staging bundle a previous `w` wrote for this hostname, if there is
    /// one. **Its operator credential is reused**, so a repeated or refused
    /// run adds no second operator key to the peers file.
    pub staged_before: Option<&'a str>,
    /// **The fleet server's own fleet-operator public key**, hex, when this
    /// machine is a MEMBER joining a known Fleet — read from the Armada
    /// Registry by the caller (`cmd/rivet-provision`'s `write_staging`,
    /// [`rivetos_register::armada::Server::operator_key`]).
    ///
    /// A member authorises control-plane callers only by the keys in its own
    /// `peers`, and its `peers` names the human operator alone; without the
    /// server's fleetd key the server can list the member but every management
    /// call is rejected at the TLS handshake (`docs/PLAN.md` §183.2). When this
    /// is `Some`, [`compose`]'s member branch adds it to the member's `peers`.
    ///
    /// **`None` for a server or a stand-alone machine, and for a member whose
    /// Fleet's key is not known** (no Armada Fleet, several with no way yet to
    /// pick one, or a server provisioned before the key was recorded). In every
    /// one of those cases the member's `peers` is staged exactly as it was
    /// before this field existed — with just the human operator.
    pub operator_key: Option<&'a str>,
    /// **Fleet-root mode** — `ADR-063` §4. `Some` for a fleet server whose
    /// root is minted on the machine at first boot: the card carries this
    /// request instead of a leaf from [`Machine::authority`], which is then
    /// never read. `None` everywhere else.
    pub fleet_root: Option<&'a rivet_core::fleet_root::Request>,
}

/// Compose everything the card must carry, and rewrite the peers file.
///
/// # Errors
///
/// A sentence naming what could not be read or minted. **Nothing is written
/// until everything is composed**, so a refusal leaves no half-provisioned
/// directory behind.
pub fn compose(machine: &Machine<'_>) -> Result<Plan, String> {
    // **A stand-alone machine stages nothing.** Ruled 2026-09-07: it enables no
    // application that reads per-machine credentials, and its own register (if
    // it has one at all) is the workstation's, not a second copy on the card
    // (`ADR-034` dropped Local mode to avoid exactly that second authority).
    //
    // **Kept as a defensive, backward-compat check, `ADR-059` §8, 2026-09-22.**
    // The interview's Fleet menu no longer offers `"stand-alone"` at all
    // (`rivet_config::FleetRole::ALL` dropped it), so a fresh run can never
    // produce this value — but an old saved answers file can still carry it,
    // and it must keep meaning exactly what it always meant: no role, nothing
    // staged. Without this check that legacy value would fall through to the
    // `machine.fleet != "server"` branch below and be staged as a member,
    // which is the one thing this function must never do to it.
    if machine.fleet == "stand-alone" {
        let mut plan = Plan::empty();
        plan.notes.push(
            "  nothing staged: a stand-alone machine enables no application that reads \
             per-machine credentials, and its own register is the WORKSTATION's (ruled \
             2026-09-07)"
                .to_owned(),
        );
        return Ok(plan);
    }

    if let Some(request) = machine.fleet_root {
        return compose_fleet_root(machine, request);
    }

    let root_certificate = read(&machine.authority.join("root.crt"))?;
    let root_key = read(&machine.authority.join("root.key"))?;

    let suffixes = crate::ca::permitted_suffixes(&root_certificate).map_err(|e| e.to_string())?;
    if suffixes.is_empty() {
        return Err("this authority constrains no DNS names, so it cannot vouch for a host".into());
    }
    let names = crate::every_name_for(machine.hostname, &suffixes);
    let Some(fleet_name) = names.first().cloned() else {
        return Err("the authority permits no suffix, so this machine has no fleet name".into());
    };

    // **One leaf per machine, from the same authority.** A server and a member
    // each get their own certificate and key for their own name — minting a
    // second certificate for the SAME name is what `ADR-011` forbids, and that
    // is not this: a server and a member never share a name.
    let leaf = crate::ca::issue_from(&root_certificate, &root_key, &names, &fleet_name)
        .map_err(|e| e.to_string())?;

    let mut items = Vec::new();
    let mut notes = Vec::new();

    // **Every fleet machine's front door, staged the same way** (`.rulings/`
    // R-160 R25): its own leaf from the fleet CA, plus the public root every
    // front door checks operator certificates against.
    push(&mut items, PROXYD, TLS_CERT, leaf.certificate.as_bytes())?;
    push(&mut items, PROXYD, TLS_KEY, leaf.key.as_bytes())?;
    push(&mut items, PROXYD, TLS_ROOT, root_certificate.as_bytes())?;
    notes.push(format!(
        "  staged for {PROXYD}: {TLS_CERT}, {TLS_KEY}, {TLS_ROOT} — its own leaf certificate \
         from the fleet CA, plus the public root"
    ));
    notes.push(format!(
        "  the certificate answers to {} and expires with the authority's own terms",
        names.join(", ")
    ));

    if machine.fleet != "server" {
        // A member's front door is everything it gets in `items`: no
        // `rivet-fleetd` files, no register. Those describe the one machine
        // that runs the fleet register, and a member runs none of it.
        //
        // **But a member must TRUST its fleet server's control plane**, or the
        // server can list it and never manage it (`docs/PLAN.md` §183.2). The
        // server dials members presenting its per-server fleet-operator key, so
        // that key must be in the member's `peers`. This mirrors the server
        // branch below, which adds a machine's OWN operator key to its OWN
        // peers — here the key is the SERVER's, threaded in from the Armada by
        // the caller, and the peers file is the member's.
        //
        // **Only when both are known**, and additive-only, for the reasons
        // `add_peer` documents (a peers file is the whole allow-list, and
        // dropping a line from a headless board's card is a lockout): with no
        // fleet key (unknown Fleet, or a server provisioned before the key was
        // recorded) or no identity directory to write, the member's peers is
        // staged exactly as it was before this existed — the human operator
        // alone.
        match (machine.operator_key, machine.identity) {
            (Some(operator_key), Some(dir)) => {
                let peers_at = dir.join(rivetos_tls::PEERS_FILE);
                let added = add_peer(&peers_at, operator_key)?;
                notes.push(if added {
                    format!(
                        "  {}: added this member's fleet server's operator key. It is what lets \
                         the server drive this member over the control plane, and it takes effect \
                         only on a boot — rivet-mcpd builds its pin list before it binds",
                        peers_at.display()
                    )
                } else {
                    format!("  {}: already names the fleet server's key; unchanged", peers_at.display())
                });
            }
            (Some(_), None) => notes.push(
                "  the fleet server's operator key is known but no identity directory was given, \
                 so it was not added to any peers file. The server will be able to list this \
                 member but not manage it until the key is in its peers (docs/PLAN.md 183.2)."
                    .to_owned(),
            ),
            (None, _) => notes.push(
                "  no fleet server operator key was supplied for this member, so its peers names \
                 only the human operator (unchanged from before this staging step existed). If \
                 this member joins a Fleet, its server cannot manage it until that Fleet's \
                 operator key reaches this member's peers (docs/PLAN.md 183.2)."
                    .to_owned(),
            ),
        }
        return Ok(Plan { items, notes, workstation_leaf: Some((fleet_name, leaf)) });
    }

    // --- Everything below is staged for a fleet server only. ---

    // **The operator credential `rivet-fleetd` asks machines with**, and the
    // one nobody had noticed. Without it the portal lists machines and can
    // drive none of them: every page answers *"this machine did not answer"*.
    // A fleet server asks ITSELF over the control plane like anything else —
    // `/run/rivet/query` is `0600 root:root` and serves only `status` — so
    // there is deliberately no self-shortcut for it to fall back on.
    //
    // **Reused from this machine's previous staging bundle when it has one.**
    // Every `w` used to mint a fresh credential, and the peers file below
    // gained a second key on every repeated or refused run.
    let operator = operator_for(machine, &fleet_name, &mut notes)?;

    push(&mut items, FLEETD, SERVER_CERTIFICATE, leaf.certificate.as_bytes())?;
    push(&mut items, FLEETD, SERVER_KEY, leaf.key.as_bytes())?;
    push(&mut items, FLEETD, AUTHORITY, root_certificate.as_bytes())?;
    push_operator(&mut items, &operator)?;

    notes.push(format!(
        "  staged for {FLEETD}: {SERVER_CERTIFICATE}, {SERVER_KEY}, {AUTHORITY}, \
         {OPERATOR_SEED}, {OPERATOR_CERTIFICATE}"
    ));

    register_and_peers(machine, &operator, &mut items, &mut notes)?;
    Ok(Plan {
        items,
        notes,
        workstation_leaf: Some((fleet_name, leaf)),
    })
}

/// **A fleet-root mode fleet server's card** — `ADR-063` §4.
///
/// Everything a server's card carries except what a workstation root would
/// have signed: no leaf, no key, no root. In their place, one entry addressed
/// to PID 1 (`rivet_core::fleet_root`), which mints the root and the leaf on
/// first boot and hands them to `rivet-proxyd` and `rivet-fleetd` under the
/// same names this module would have. The fleet server's operator
/// credential, its one-record register and its `peers` entry are staged
/// exactly as for any server, because none of them is a browser certificate.
///
/// No workstation authority is read and no workstation copy of a leaf is
/// written: there is no leaf here to copy.
fn compose_fleet_root(
    machine: &Machine<'_>,
    request: &rivet_core::fleet_root::Request,
) -> Result<Plan, String> {
    if machine.fleet != "server" {
        return Err(format!(
            "fleet-root mode mints the fleet's root on its FLEET SERVER, and this machine is a \
             {}. A member of a fleet-root mode fleet is issued its certificate by its fleet \
             server (ADR-063 section 4.4)",
            machine.fleet
        ));
    }
    if request.host != machine.hostname {
        return Err(format!(
            "the fleet-root request names host {:?} and this machine is {:?}",
            request.host, machine.hostname
        ));
    }
    let names = request.names();
    let Some(fleet_name) = names.first().cloned() else {
        return Err("the fleet-root request composes no name".to_owned());
    };

    let mut items = Vec::new();
    let mut notes = Vec::new();
    push(
        &mut items,
        rivet_core::fleet_root::REQUEST_APPLICATION,
        rivet_core::fleet_root::REQUEST_PATH,
        request.render().as_bytes(),
    )?;
    notes.push(format!(
        "  fleet-root mode: NO root and NO leaf from this workstation. The card carries a \
         request, and on first boot the machine mints fleet {:?}'s root, constrained to {}, and \
         its own certificate for {} (ADR-063 section 4.2)",
        request.fleet,
        request.suffixes.join(", "),
        names.join(", ")
    ));

    let operator = operator_for(machine, &fleet_name, &mut notes)?;
    push_operator(&mut items, &operator)?;
    notes.push(format!(
        "  staged for {FLEETD}: {OPERATOR_SEED}, {OPERATOR_CERTIFICATE}; its {SERVER_CERTIFICATE}, \
         {SERVER_KEY} and {AUTHORITY} are minted on the machine"
    ));
    register_and_peers(machine, &operator, &mut items, &mut notes)?;
    Ok(Plan {
        items,
        notes,
        workstation_leaf: None,
    })
}

/// The operator credential `rivet-fleetd` asks machines with: the previous
/// bundle's when it carries one, else freshly minted.
fn operator_for(
    machine: &Machine<'_>,
    fleet_name: &str,
    notes: &mut Vec<String>,
) -> Result<SelfSigned, String> {
    Ok(match machine.staged_before.and_then(staged_operator) {
        Some(previous) => {
            notes.push(format!(
                "  reused the {FLEETD} operator credential this machine's previous staging bundle \
                 carries"
            ));
            previous
        }
        None => mint_self_signed(&format!("{fleet_name} fleet operator"))?,
    })
}

/// The operator credential's two files, as hex text.
fn push_operator(items: &mut Vec<Staged>, operator: &SelfSigned) -> Result<(), String> {
    push(
        items,
        FLEETD,
        OPERATOR_SEED,
        format!("{}\n", rivetos_tls::encode_hex(&operator.seed)).as_bytes(),
    )?;
    push(
        items,
        FLEETD,
        OPERATOR_CERTIFICATE,
        format!("{}\n", rivetos_tls::encode_hex(&operator.certificate_der)).as_bytes(),
    )
}

/// A fleet server's one-record register, and its operator key in `peers`.
fn register_and_peers(
    machine: &Machine<'_>,
    operator: &SelfSigned,
    items: &mut Vec<Staged>,
    notes: &mut Vec<String>,
) -> Result<(), String> {
    // **The register, and the pin in it is the machine's own control-plane key
    // read out of the certificate on the card.** Without a pin a record is one
    // `rivet-fleet` refuses to dial, by name.
    match machine.identity {
        Some(dir) => {
            let pin = device_pin(dir)?;
            let hex_pin = rivetos_tls::encode_hex(&pin);
            let operator_key = rivetos_tls::encode_hex(&operator.public);
            for (path, text) in register(machine, &pin, &operator_key)? {
                push(items, FLEETD, &path, text.as_bytes())?;
            }
            notes.push(format!(
                "  staged for {FLEETD}: a ONE-record register — its own — with pin {hex_pin}"
            ));

            let peers_at = dir.join(rivetos_tls::PEERS_FILE);
            let added = add_peer(&peers_at, &operator_key)?;
            notes.push(if added {
                format!(
                    "  {}: added this fleet server's own operator key. It is what lets the \
                     machine accept its own fleet server as a caller, and it takes effect only \
                     on a boot — rivet-mcpd builds its pin list before it binds",
                    peers_at.display()
                )
            } else {
                format!("  {}: already names that key; unchanged", peers_at.display())
            });
        }
        None => {
            // **Named rather than guessed past.** A register with no pin is one
            // nothing can dial, and a peers file that does not name the fleet
            // server's key is a fleet server that cannot reach its own machine
            // — repairable only on a boot, on a headless board.
            notes.push(
                "  NO REGISTER AND NO PEERS ENTRY: no identity directory was given, so this tool \
                 cannot read the machine's own control-plane public key or write the key the \
                 machine must accept. The card will boot a fleet server that serves an EMPTY \
                 register and cannot drive itself. Answer the Identity question with the \
                 directory `cargo rivet setup` wrote."
                    .to_owned(),
            );
        }
    }
    Ok(())
}

/// An Ed25519 self-signed leaf and the seed behind it.
pub(crate) struct SelfSigned {
    pub(crate) seed: [u8; rivetos_tls::KEY_BYTES],
    pub(crate) certificate_der: Vec<u8>,
    pub(crate) public: rivetos_tls::Pin,
}

/// Mint a control-plane identity: the one `rivet-fleetd` presents, and a
/// machine's own (`crate::identity::mint`).
///
/// **The same shape `cargo rivet identity new` mints**, deliberately: a
/// self-signed Ed25519 leaf, never an authority, because `ADR-035` §7 makes the
/// whole certificate the thing a peer pins and `rivetos-tls` refuses a chain
/// outright.
pub(crate) fn mint_self_signed(subject: &str) -> Result<SelfSigned, String> {
    let pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)
        .map_err(|e| format!("cannot generate an Ed25519 keypair: {e}"))?;
    let mut params = rcgen::CertificateParams::new(vec![subject.to_owned()])
        .map_err(|e| format!("cannot build the certificate's parameters: {e}"))?;
    let mut name = rcgen::DistinguishedName::new();
    name.push(rcgen::DnType::CommonName, subject.to_owned());
    params.distinguished_name = name;
    params.is_ca = rcgen::IsCa::NoCa;
    let certificate = params
        .self_signed(&pair)
        .map_err(|e| format!("cannot self-sign the certificate: {e}"))?;

    let der = certificate.der().to_vec();
    let public = rivetos_tls::public_key_in_certificate(&der).ok_or_else(|| {
        "the certificate just minted carries no Ed25519 public key, which means rivetos-tls and \
         rcgen disagree about the encoding"
            .to_owned()
    })?;
    Ok(SelfSigned {
        seed: seed_of(&pair.serialize_der())?,
        certificate_der: der,
        public,
    })
}

/// The operator credential in a staging bundle, when both halves read back
/// and are one key. `None` for anything else, which mints a fresh one.
fn staged_operator(bundle: &str) -> Option<SelfSigned> {
    let items = rivet_core::staging::parse(bundle).ok()?;
    let hex_at = |path: &str| {
        items
            .iter()
            .find(|i| i.application == FLEETD && i.path == path)
            .and_then(|i| core::str::from_utf8(&i.bytes).ok())
            .and_then(|text| decode_hex(text.trim()))
    };
    let seed = <[u8; rivetos_tls::KEY_BYTES]>::try_from(hex_at(OPERATOR_SEED)?).ok()?;
    let certificate_der = hex_at(OPERATOR_CERTIFICATE)?;
    let public = rivetos_tls::public_key_in_certificate(&certificate_der)?;

    // A PKCS#8 v1 wrapper around the seed, so the public half can be derived
    // and compared: a mismatched pair is `KeyMismatch` on the machine.
    let mut pkcs8 = vec![
        0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04,
        0x20,
    ];
    pkcs8.extend_from_slice(&seed);
    let pair = rcgen::KeyPair::from_pkcs8_der_and_sign_algo(&pkcs8.into(), &rcgen::PKCS_ED25519)
        .ok()?;
    (pair.public_key_raw() == public.as_slice()).then_some(SelfSigned {
        seed,
        certificate_der,
        public,
    })
}

/// Hex text into bytes; `None` for an odd length or a non-hex character.
fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| text.get(i..i + 2).and_then(|pair| u8::from_str_radix(pair, 16).ok()))
        .collect()
}

/// The seed inside a PKCS#8 blob.
///
/// **`rcgen` returns PKCS#8 v2, which appends the PUBLIC key after the private
/// one**, so the last thirty-two bytes are the wrong half — and writing them
/// produces a seed `rustls` rejects with `KeyMismatch` the first time anything
/// tries to serve with it. The private key is the thirty-two bytes after the
/// `04 22 04 20` that opens its OCTET STRING.
///
/// The same four bytes `cmd/cargo-rivet`'s `identity` module looks for, and the
/// duplication is deliberate rather than overlooked: that crate is a binary with
/// no library target, so this cannot call it, and the alternative — a shared
/// crate for four constants — is a dependency edge for a comment.
fn seed_of(pkcs8: &[u8]) -> Result<[u8; rivetos_tls::KEY_BYTES], String> {
    let at = pkcs8
        .windows(4)
        .position(|w| w == [0x04, 0x22, 0x04, 0x20])
        .ok_or("rcgen produced a PKCS#8 blob with no Ed25519 private key octet string in it")?
        + 4;
    pkcs8
        .get(at..at + rivetos_tls::KEY_BYTES)
        .and_then(|s| <[u8; rivetos_tls::KEY_BYTES]>::try_from(s).ok())
        .ok_or_else(|| "rcgen's PKCS#8 blob ends before the seed does".to_owned())
}

/// The machine's own control-plane public key, hex, out of its certificate.
///
/// **`device.seed` is never opened.** The public half is in the certificate,
/// which is what `cargo rivet identity show` reads and for the reason it gives:
/// a tool that opens a private key it does not need is one whose failures can
/// leak it.
///
/// `pub(crate)` since 2026-09-13 (`.rulings/` R-178 Q7): `main`'s `w` key also
/// reads this, independent of whether a role's staging carries anything with
/// it — the pin it returns is `ADR-049` §8.1's identifier source for the
/// register the OPERATOR enrolls this machine in, which R-178 P4 moved out of
/// this process entirely (`cargo rivet provision`, after the card is written).
pub(crate) fn device_pin(identity: &Path) -> Result<rivetos_tls::Pin, String> {
    let path = identity.join(rivetos_tls::CERTIFICATE_FILE);
    let der = std::fs::read(&path).map_err(|e| {
        format!(
            "{}: {e}. This is the card's own identity, as `cargo rivet setup` writes it; without \
             it the register record would carry no pin and nothing could dial the machine",
            path.display()
        )
    })?;
    rivetos_tls::public_key_in_certificate(&der).ok_or_else(|| {
        format!(
            "{} carries no Ed25519 public key, so it is not a device certificate",
            path.display()
        )
    })
}

/// The one-record register, rendered in `rivetos_register`'s own layout.
///
/// **Rendered by that crate and not by this one.** `cmd/rivet-fleetd` reads it
/// with `rivetos_register::store::load`, and a second hand-written spelling of
/// the format is `ADR-011`'s failure in a file whose unreadability reports
/// nothing.
fn register(
    machine: &Machine<'_>,
    pin: &rivetos_tls::Pin,
    operator_key: &str,
) -> Result<Vec<(String, String)>, String> {
    let mut device = rivetos_register::Device {
        // **Derived from the pin, so it is stable and opaque.** Re-provisioning
        // the same machine produces the same identifier rather than a second
        // record for one board, and nothing about the machine can be read out
        // of it — `ADR-025` §4 wants both.
        id: rivet_core::identifier::from_entropy(pin),
        name: machine.hostname.to_owned(),
        // `rivet-fleetd` finds its own control plane by this word.
        role: Some(rivet_core::fleet_as_code::SERVER_ROLE.to_owned()),
        pin: Some(rivetos_tls::encode_hex(pin)),
        listen: Some(machine.listen.to_owned()),
        ..rivetos_register::Device::default()
    };
    device.interfaces.insert(
        "eth0".to_owned(),
        rivetos_register::Interface {
            method: machine.method.to_owned(),
            address: (machine.method == "static" && !machine.address.is_empty())
                .then(|| machine.address.to_owned()),
            gateway: (machine.method == "static" && !machine.gateway.is_empty())
                .then(|| machine.gateway.to_owned()),
        },
    );

    // **A relative root**, because what is being rendered is the layout rather
    // than a place on this workstation: the paths become entries in the staging
    // bundle and the machine joins them onto its own data directory.
    let root = PathBuf::from(REGISTER_DIR);
    let mut reg = rivetos_register::Register::empty(&root);
    reg.fleet.control_plane_keys = vec![operator_key.to_owned()];
    reg.devices.insert(device.id.clone(), device);

    let rendered = rivetos_register::render(&reg).map_err(|e| e.to_string())?;
    rendered
        .into_iter()
        .map(|(path, text)| {
            let relative = path
                .to_str()
                .ok_or_else(|| "the register rendered a path that is not text".to_owned())?
                .replace('\\', "/");
            Ok((relative, text))
        })
        .collect()
}

/// Put a key in a peers file, keeping every key already in it.
///
/// **Only ever adds.** A peers file is the whole list of who may reach a
/// machine's control plane; dropping a line from it on a headless board is a
/// lockout with no recovery but a new card, so this reads what is there, adds
/// what is missing, and rewrites.
///
/// Answers whether anything was added.
pub(crate) fn add_peer(path: &Path, key: &str) -> Result<bool, String> {
    let existing = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if existing
        .lines()
        .any(|line| line.trim().eq_ignore_ascii_case(key))
    {
        return Ok(false);
    }
    let mut out = existing;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(key);
    out.push('\n');
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(path, out).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(true)
}

fn push(items: &mut Vec<Staged>, app: &str, path: &str, bytes: &[u8]) -> Result<(), String> {
    items.push(Staged::new(app, path, bytes.to_vec()).map_err(|e| e.to_string())?);
    Ok(())
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
#[path = "stage_tests.rs"]
mod tests;
