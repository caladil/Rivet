//! **`rivet-provision`** — the one command-line tool meant for a person.
//!
//! It asks what a machine should be, checks the answers as they are given,
//! validates the whole document against the schema the board itself refuses
//! with, and writes it out.
//!
//! # What it is for, in one line
//!
//! `tools/card/provisioning.example.toml` names the defect: *"`cmd/rivet-image`
//! does not validate this… the refusal comes from the board, on first boot,
//! which is the wrong end."* This is that end being made right.
//!
//! # `RIVET_AT` — the root every path in this tool hangs off, 2026-09-14
//!
//! **Found building `cmd/cargo-rivet/tests/pty_interview.rs`** (the
//! transcript is `r178-p6-pty-proof/README.md` in the maintainer's scratchpad,
//! outside this repository): `cargo rivet provision --at DIR` set
//! `RIVET_FLEET_REPOSITORY` on this process and nothing else, so
//! [`authority_directory`], [`save::directory`] (the saved-answers
//! notebook, the composed document, the `ADR-049` staging bundle) and
//! [`default_identity`] all kept reading `HOME`/`USERPROFILE` directly —
//! this HOST's real ones, `--at` or no `--at`. A scratch `--at` run offered
//! `resume which?` naming a real saved notebook from the maintainer's own
//! machine before a single keystroke was sent.
//!
//! [`rivet_root`] is the fix: `RIVET_AT`, set by `cargo rivet provision`'s
//! `SubprocessInterview` to the SAME root it already resolves for its own
//! credentials survey and image verification, is read FIRST — falling back
//! to `$HOME/.rivet`/`$USERPROFILE\.rivet` only when it is unset, which is
//! exactly this tool's own behaviour when run directly, with nobody setting
//! the variable. Every function that used to read `HOME`/`USERPROFILE` for
//! itself now goes through [`rivet_root`] instead, so there is exactly one
//! place left that reads either variable.

mod backup;
mod browser;
mod ca;
mod field;
mod identity;
mod save;
mod screen;
mod stage;
mod stage_document;
mod versions;
mod zones;

use std::io::Write as _;

use field::{Field, Kind};
use rivet_config::interview;
use screen::{Colour, Row};

/// **Mint, write `browser.pfx`, and do not open the certificate store.**
///
/// The flag that makes `SPEC-05` §4 rehearsable. `browser::place` shells out to
/// `certutil`, which Windows resolves from its system directory — so taking it
/// off `PATH` does not stop it, and until this flag existed every practice run
/// of the ceremony left a certificate in the operator's real Personal store.
const NO_INSTALL: &str = "--no-install";

/// **Compose a server certificate's names literally**, instead of from the
/// authority's own recorded suffixes.
///
/// `ca server rivet-01.local <dir>` once minted a leaf with a *single* name and
/// silently replaced a three-name one, so only `rivet-01.local` validated and
/// the fleet's two other suffixes warned with nothing saying why
/// (`Fleet-Bring-Up-Findings.md` §3.3). Composing is therefore the default and
/// a literal list is opt-in — a person who means exactly one name says so.
const NAMES: &str = "--names";

/// **Overwrite what a backup or a restore would otherwise refuse to touch.**
///
/// Shared by `ca backup` and `ca restore`, because both refuse the same way:
/// a file already at the destination is named and the run stops, unless this
/// is given. `SPEC-05` §4.
const REPLACE: &str = "--replace";

/// What `--help` says about the two verbs `clap` cannot see.
///
/// **`ca` and `ca server` are matched before the argument parser**, because
/// Ignition's surface does not know them and `clap` refuses an argument it was
/// not told about. The cost of that is that they were invisible to `--help`,
/// which is where a person looks for a flag — so the help text carries them by
/// hand rather than not carrying them at all.
const CA_HELP: &str = "\
Verbs (matched before the parser, so they do not appear above):

  rivet-provision ca <suffixes> <fleet-name> <directory> [operator-name] [--no-install]
          Mint the fleet's certificate authority, its root, and your own browser
          identity. Files the identity into your Windows Personal store.

          --no-install  Write browser.pfx and DO NOT touch the certificate
                        store. This is how the ceremony is rehearsed: certutil
                        lives in the system directory, so there is no other way
                        to run `ca` without adding a certificate to the store
                        you are rehearsing for.

  rivet-provision ca server <hostname> <directory> [--names <list>]
          Mint a fleet server's certificate. By DEFAULT it composes
          <hostname>.<suffix> for every suffix the authority permits, read out
          of root.crt. --names takes a literal comma-separated list instead,
          for somebody who means exactly those names and no others.

  rivet-provision ca backup <directory> [--replace]
          Copy the authority (~/.rivet/ca) to <directory>. REQUIRED: there is
          no default location for storing secret keys. Reports every file
          copied, with its size. Refuses to overwrite a backup already at
          <directory> unless --replace is given.

  rivet-provision ca restore <directory> [--replace] [--no-install]
          Copy the authority back from a backup at <directory>, and
          re-install root.crt into the Windows Root store and browser.pfx
          into Personal — the inverse of `rivet-reset`, not of a plain copy.
          Refuses to restore over an authority already at ~/.rivet/ca unless
          --replace is given. --no-install writes the files and does not
          touch the certificate store; use it for every rehearsal.

  rivet-provision stage --document <path> --identity <dir> [--at <dir>]
                        [--fleet-root <fleet> [--suffixes <list>]] [--passkeys <file>]
          Build the ADR-049 staging bundle for a server or member document
          that has none, without the interview: the role, hostname and
          addressing come from the document, the authority from <at>/ca. It
          is written beside the document and listed by name and size, never
          by content. cargo rivet provision and write-image run it themselves
          when a server or member document has no bundle. --fleet-root makes
          a server's card carry a request instead: the machine mints the
          fleet's root and its own certificate on first boot (ADR-063).
          --passkeys carries a proxy's passkeys file to the new card.

Run with no arguments for the menu. On a workstation with no authority it
first asks whether you are starting a new fleet or joining an existing one,
and mints nothing until you answer (ADR-063 section 8). A new fleet's root
is minted here (1), or on its fleet server's first boot (3, fleet-root mode).

  rivet-provision --new-fleet
          Answer that question in advance: this is a new fleet, so mint its
          authority. Without a terminal to ask at, the menu refuses rather than
          mint unless this is given.";

/// **"This is a new fleet"**, said in advance — the first-provision guard's
/// answer for a run with no terminal to ask at (`ADR-063` §8).
///
/// Taken out of the arguments before the parser sees them, for the reason the
/// `ca` verb is matched early: Ignition's surface does not know it, and this
/// crate names no argument parser of its own to teach.
const NEW_FLEET: &str = "--new-fleet";

/// The DNS suffix a fleet has when nobody says otherwise.
///
/// **`local`, because that is what Rivet actually serves today** —
/// `rivet-mdnsd` announces `<hostname>.local` and there is no DNS server in
/// this project. It is the default rather than the only option: the maintainer
/// ruled 2026-09-06 that the authority must be able to mint for an ordinary DNS
/// environment, since a root's name constraint cannot be widened afterwards.
const DEFAULT_SUFFIX: &str = "local";

/// The IETF's reserved namespace for home networks — **RFC 8375**.
///
/// # Why this is a default and `.lan` is not
///
/// Both are used for private networks and only one of them is *safe to put in a
/// name constraint that lives ten years*.
///
/// **`home.arpa` can never be delegated.** RFC 8375 reserves it permanently, so
/// an authority constrained to a subdomain of it can never, under any future
/// decision by anybody, be made to vouch for a name on the public internet.
///
/// **`.lan` is merely undelegated today.** It is a widely-honoured de-facto
/// convention and it is not in IANA's special-use registry; ICANN's
/// name-collision work flags it precisely because so much private
/// infrastructure assumes it. If it were ever delegated, a root constrained to
/// it would become a root that can vouch for real names — the exact failure
/// `ca`'s name constraints exist to prevent, on the one field that cannot be
/// re-cut.
///
/// The asymmetry decides it: including `.lan` buys a convenience only for
/// somebody whose router already serves it, and costs a small permanent risk to
/// everybody. **So it is typeable and never offered** — a person whose router
/// serves `.lan` names it at minting time, in one word.
///
/// *(Ruled by the maintainer 2026-09-06, who raised both and was iffy about the
/// second for the right reason.)*
const HOME_ARPA: &str = "home.arpa";

/// The suffixes offered when a person is minting an authority for `fleet`.
///
/// **`local` and `<fleet>.home.arpa`.** The first is what Rivet actually serves
/// — `rivet-mdnsd` announces `<hostname>.local` and nothing else has to run.
/// The second serves nothing today and is the point: it is the name this fleet
/// will want the day it has a DNS server, and a name constraint cannot be
/// widened afterwards.
///
/// **The fleet's own name is slugified rather than trusted.** It is free text —
/// `my-fleet`, but also `Someone's fleet` — and a DNS label is not. A name that
/// cannot produce a label yields `local` alone rather than an invalid suffix
/// the authority would refuse.
fn default_suffixes(fleet: &str) -> Vec<String> {
    let mut out = vec![DEFAULT_SUFFIX.to_owned()];
    if let Some(label) = dns_label(fleet) {
        out.push(format!("{label}.{HOME_ARPA}"));
    }
    out
}

/// `fleet` as a single DNS label, or `None` if nothing usable is left.
///
/// Lower-case, ASCII letters, digits and hyphens; anything else becomes a
/// hyphen, runs collapse, and the ends are trimmed — because a label may not
/// begin or end with one. Capped at 63 bytes, which is the label limit every
/// resolver enforces.
fn dns_label(fleet: &str) -> Option<String> {
    let mut out = String::new();
    for c in fleet.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-');
    if out.is_empty() {
        return None;
    }
    Some(out.chars().take(63).collect::<String>().trim_end_matches('-').to_owned())
}

/// Read a comma-separated suffix list, in the shape [`ca::Authority::root`]
/// takes.
///
/// **Separators are commas OR whitespace**, because a person typing two of
/// something types it the way they think of it and being refused over a space
/// is the kind of friction that makes a person guess instead of read. Empty
/// entries fall out here rather than reaching the authority, which refuses
/// them — a trailing comma is a typo, not a request to constrain nothing.
fn split_suffixes(given: &str) -> Vec<String> {
    given
        .split([',', ' ', '\t', '\n', '\r'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// What the fleet is called when nobody says otherwise.
///
/// **A logical label, not a hostname.** It is the authority's `commonName` —
/// what a trust store shows beside the root certificate — and no machine is
/// ever reached by it.
const DEFAULT_FLEET: &str = "my-fleet";

/// What the operator is called when nobody says otherwise.
///
/// **`fleet-owner`**, ruled by the maintainer 2026-09-06: the name a person is
/// offered should be the role they are about to take.
///
/// It is **not** cosmetic. `rivet-proxyd`'s index prints it as *"Authenticated
/// as: …"* and the journal records it as the principal of every mutating
/// operation, which is the only thing that will ever distinguish one operator
/// from another. A person typing their own name here is doing something real,
/// spaces included.
const DEFAULT_OPERATOR: &str = "fleet-owner";

/// The questions, in the order a person answers them —
/// **generated from `rivet_config::schema::DOCUMENT`**, not a hand-written
/// list. `docs/PLAN-Feature-Set-2026-09-12.md` §5.2 item 3, `.rulings/`
/// R-162, Wave 3b. `ADR-007` §7's declaration order already reproduces
/// this list's own order — `hostname`, `network`, `mode`, `fleet-role`,
/// `timezone`, `kiosk` — so `rivet_config::interview::generate` produces the
/// identical sequence this function used to spell out by hand: **Hostname**
/// first (it is the name that will go in a fleet server's certificate and
/// the name typed into a browser afterwards; changing it later is not a
/// field edit), then **Addressing/Address/Gateway**, **Mode**, **Fleet**,
/// **Timezone**, **Auto-run**. Adding a schema key in the right position is
/// now the whole of adding a question — see `rivet_config::interview` for
/// what is generated generically and what is curated (`network` and
/// `[kiosk]`'s own nested questions), and `interview_tests.rs` for the test
/// that pins this exact order and breaks if it drifts.
fn questions() -> Vec<Field> {
    let hostname = suggest_hostname();
    let mut fields: Vec<Field> = interview::generate(rivet_config::schema::DOCUMENT)
        .into_iter()
        .map(|q| match q.answer {
            // **Fleet's untouched default stays "stand-alone", `ADR-059` §8,
            // 2026-09-22.** `Field::choice`'s own rule is "the first is the
            // default" — fine for `Addressing` (dhcp) and `Mode` (headless),
            // wrong here now that `rivet_config::FleetRole::ALL` dropped the
            // word: `of` is exactly `{"server", "member"}`, and defaulting to
            // whichever sorts first would silently turn a `w` where the
            // operator never opened this question into a fleet server or
            // member. `"stand-alone"` is not one of the two offered choices
            // any more, but it is still the one spelling `listen_for`,
            // `services_for` and `stage::compose` already treat as exactly
            // "no role" for a legacy document — reusing it here as the
            // pre-filled value keeps an untouched Fleet question producing
            // the same roleless document it always has, with no new sentinel
            // to teach those functions.
            interview::Answer::Choice(of) if q.key == "fleet-role" => {
                let mut field = Field::choice(q.label, of, q.description);
                field.value = "stand-alone".to_owned();
                field.default = "stand-alone".to_owned();
                field
            }
            interview::Answer::Choice(of) => Field::choice(q.label, of, q.description),
            // **`hostname`'s default is the one runtime fact no schema
            // carries** — a suggestion computed from this process, not a
            // property of the document. Every other generated text field
            // starts blank, exactly as it always has.
            interview::Answer::Text if q.key == "hostname" => {
                Field::text(q.label, &hostname, q.description)
            }
            interview::Answer::Text => Field::text(q.label, "", q.description),
        })
        .collect();

    // **`Identity` is appended, not generated, and stays LAST** — `ADR-049`
    // and `SPEC-05` §4: the numbers 1 through 8 above are in every note and
    // every rehearsal script, and a ninth field placed anywhere else would
    // renumber all of them for no gain. It names a directory on THIS
    // WORKSTATION — never a fact about the document `rivet-config` parses —
    // so it has no schema key to generate from. Its default is this machine's
    // own directory, and follows the Hostname answer (`crate::identity`).
    fields.push(Field::text(
        "Identity",
        &default_identity(&hostname),
        "The directory holding THIS MACHINE's device.seed, device.crt and peers — what \
         `cargo rivet provision` puts on the card. The default is ~/.rivet/identity/<hostname>, \
         one identity per machine, and it follows the hostname until you type another. A \
         directory holding no identity is MINTED at `w`: a fresh key, and a peers file naming \
         your operator identity. An identity whose key is already on another machine's \
         register record is refused at `w`, naming that record. device.seed is never opened \
         here. The certificate gives the machine's control-plane key for its register record, \
         and the peers file gains the fleet server's own operator key. Blank on a `server` \
         writes a card whose fleet portal serves an empty register and cannot drive its own \
         machine.",
    ));

    // **`Nameservers` is appended after `Identity`, for `Identity`'s reason**:
    // numbers 1 through 9 are in every note and rehearsal script, and a field
    // placed beside `Gateway` would renumber five of them. Being last is also
    // what lets it be hidden under dhcp without moving any other number — see
    // [`asked`]. It is curated here rather than generated because
    // `[network.resolver]` is a reserved sub-table of `network`, not a
    // property of an interface, so `rivet_config::interview` has no schema
    // property to generate it from.
    fields.push(Field::text(
        NAMESERVERS,
        "",
        "The nameservers this machine asks, in order: up to three IPv4 addresses, because musl \
         asks no more. Asked only for a static address; under dhcp the lease names them. The \
         default is the gateway, and a bare Return at the prompt takes it. Written as \
         [network.resolver] nameservers. Without it a static machine's every lookup fails inside \
         whichever application made it.",
    ));
    fields
}

/// The label of the one question asked only for a static address, and the
/// key it is saved under in the answers notebook (lower-cased).
const NAMESERVERS: &str = "Nameservers";

/// **Whether the menu asks `label` at all, given the answers so far.**
///
/// `Nameservers` is asked only when `Addressing` is `static`. A dhcp machine's
/// resolver is filled from its lease, and only while the document's
/// `[network.resolver]` is silent (`rivet_config::resolver`), so asking would
/// compose a table that overrides the lease with a guess. Every other field is
/// always asked.
fn asked(fields: &[Field], label: &str) -> bool {
    label != NAMESERVERS || value_of(fields, "Addressing") == "static"
}

/// **The Nameservers default follows the gateway**, until the operator types
/// something else — as the Gateway default follows the address.
///
/// The default is always the gateway; the value follows only while it is
/// still the old default (or empty), so an answer the operator typed is never
/// overwritten. A gateway that is not an address yet is not followed: a
/// suggestion derived from nonsense is worse than none.
fn follow_gateway(fields: &mut [Field]) {
    let gateway = value_of(fields, "Gateway");
    if field::ipv4(&gateway).is_err() {
        return;
    }
    let Some(f) = fields.iter_mut().find(|f| f.label == NAMESERVERS) else {
        return;
    };
    let untouched = f.value.is_empty() || f.value == f.default;
    f.default.clone_from(&gateway);
    if untouched {
        f.set(&gateway, field::nameservers);
    }
}

/// **What an answer typed at a field's prompt means**, before its check.
///
/// A number picks from `offered`, if the field offers anything. **A bare
/// Return at `Nameservers` takes its default**, the gateway, and is the one
/// field where it does: everywhere else an empty answer is an answer, and
/// `Auto-run` in particular is cleared by one.
fn chosen_answer(f: &Field, typed: &str, offered: &[String]) -> String {
    if typed.is_empty() && f.label == NAMESERVERS {
        return f.default.clone();
    }
    typed
        .parse::<usize>()
        .ok()
        .and_then(|n| n.checked_sub(1))
        .and_then(|i| offered.get(i))
        .cloned()
        .unwrap_or_else(|| typed.to_owned())
}

/// **The facts `cargo rivet provision`'s orchestrator needs from this
/// interview**: the document it just wrote, at `~/.rivet/provision/.latest-run.txt`
/// — the same directory `SPEC-05` §4 already writes the document itself
/// into — and, on a second line, this card's own control-plane pin as hex,
/// when one could be read. Plan §5.2 item B, `.rulings/` R-162, Wave 3b;
/// the pin line is `.rulings/` R-178 Q7/Q10: the orchestrator derives
/// `ADR-049` §8.1's register identifier from it, and has no other route to
/// this workstation fact, which lives only in the "Identity" answer this
/// interview never writes into the document itself.
///
/// `pin_hex` is `None` for a stand-alone machine, one whose Identity answer
/// named nothing readable, or a build that never asked (README §2's own
/// `--identity`/`--pin` flags on `cargo rivet provision` are the fallback for
/// exactly this).
///
/// **Best-effort, on purpose.** A workstation with neither `HOME` nor
/// `USERPROFILE` already failed to write the document itself (`write_document`
/// resolves the same way), so there is nothing new to refuse here; a write
/// that fails for some other reason costs the orchestrator a clearer error
/// on its own next run, never the document this interview already wrote
/// successfully.
/// **Wave C3.** `identity_dir` is the "Identity" answer, verbatim, when it
/// named a directory — the orchestrator's only route to this workstation
/// fact (see the module doc above `identity_value`'s own read). Without it,
/// `write_image::run`'s widened injection (`.rulings/` R-178 item 1) falls
/// back to `credentials::Survey`'s own default (`~/.rivet/identity/board`),
/// which is right for the ordinary case and silently wrong for an operator
/// who typed a different directory into the interview — so this line closes
/// that gap on the identical footing the pin line already stands on: best
/// effort, third rather than assumed.
fn write_breadcrumb(document_path: &std::path::Path, pin_hex: Option<&str>, identity_dir: Option<&str>) {
    // **The same directory `save::directory` writes the document and the
    // notebook into** — restated as a call rather than a second, independent
    // `HOME`/`USERPROFILE` read: the whole point of `RIVET_AT` (this
    // module's own doc) is that there is exactly one place left that reads
    // either variable.
    let Some(dir) = save::directory() else {
        return;
    };
    let mut text = format!("{}\n", document_path.display());
    if let Some(pin) = pin_hex {
        text.push_str(&format!("pin={pin}\n"));
    }
    if let Some(identity) = identity_dir {
        text.push_str(&format!("identity={identity}\n"));
    }
    let _ = std::fs::write(dir.join(".latest-run.txt"), text);
}

/// Where a new machine named `hostname` keeps its identity —
/// `~/.rivet/identity/<hostname>`, `.rulings/` R-189 D2.
///
/// **Offered as a default and checked when it is used**, not here: a person
/// provisioning a card for somebody else's board has a different directory, and
/// a tool that refused to start without this one would be wrong about whose
/// machine it is.
fn default_identity(hostname: &str) -> String {
    let Some(root) = rivet_root() else {
        return String::new();
    };
    identity::directory_for(&root, hostname).display().to_string()
}

/// **The Identity answer, checked at `w` before anything is composed** —
/// `.rulings/` R-189 D2. Answers whether `w` may go on.
///
/// A directory that holds an identity is refused if another hostname's record
/// carries its pin. Run without `cargo rivet provision`, no register is named,
/// so this says it did not look; that run's `--document --identity` checks
/// before writing a card.
fn identity_allowed(fields: &[Field]) -> bool {
    let answer = value_of(fields, "Identity");
    let answer = answer.trim();
    if answer.is_empty() {
        return true;
    }
    let dir = std::path::Path::new(answer);
    let Some(register) = identity::register_root() else {
        if !identity::is_absent(dir) {
            println!(
                "  {answer} was not checked against a register: RIVET_FLEET_REGISTER is not set. \
                 cargo rivet provision checks it before writing a card."
            );
        }
        return true;
    };
    match identity::check(dir, &value_of(fields, "Hostname"), &register) {
        Ok(()) => true,
        Err(why) => {
            println!("  refused: {why}");
            false
        }
    }
}

/// **Mint the Identity answer's directory if it holds no identity** —
/// `.rulings/` R-189 D2. Called at `w`, after the document is written and
/// before anything is staged. Answers whether `w` may go on.
fn mint_identity_if_absent(fields: &[Field]) -> bool {
    let answer = value_of(fields, "Identity");
    let answer = answer.trim();
    if answer.is_empty() || !identity::is_absent(std::path::Path::new(answer)) {
        return true;
    }
    let Some(root) = rivet_root() else {
        println!("  NOT minted: no home directory, so there is no operator identity to read");
        return false;
    };
    let dir = std::path::Path::new(answer);
    match identity::mint(dir, &root.join("identity").join("operator")) {
        Ok(pin) => {
            println!("  minted identity {}", dir.display());
            println!("    pin {}", rivetos_tls::encode_hex(&pin));
            true
        }
        Err(why) => {
            println!("  NOT minted: {why}");
            false
        }
    }
}

/// The zones and applications a person is offered **by number** on a field that
/// is not a fixed choice.
///
/// **A text field with suggestions, which `Kind::Choice` cannot express.** A
/// timezone has six hundred legal answers and a menu cannot print them; a
/// choice field would refuse every one it did not list. So the numbered list is
/// a shortcut into free text — the same ruling (.rulings/ R-346) that made
/// `dhcp`/`static` numeric, without turning the shortcut into the whole set.
fn offered_for(label: &str) -> Vec<String> {
    match label {
        "Timezone" => zones::OFFERED.iter().map(|s| (*s).to_owned()).collect(),
        // The launcher is the one application this tool can name without
        // guessing at a payload: it is what offers whatever else is installed.
        "Auto-run" => vec!["rivet-launcher".to_owned()],
        _ => Vec::new(),
    }
}

/// A name nobody has to invent.
///
/// **Shown as the default before it is accepted**, which is the maintainer's
/// rule (.rulings/ R-340): a default is visible to the user before they decide.
fn suggest_hostname() -> String {
    // Deterministic per machine would be better; this is the shape, and the
    // source of the suffix is a decision the maintainer has not made yet.
    format!("rivet-{:05x}", std::process::id() & 0xf_ffff)
}

/// The **form** of a field's answer, shown before it is typed.
///
/// Beside [`checker`] and shaped like it, deliberately: the two are one fact
/// seen from opposite ends — what will be accepted, and what to type to be
/// accepted — and a field that gains a rule should gain its sentence here in
/// the same edit.
///
/// **Empty for a field whose answer needs no shape.** A choice is picked by
/// number and a hostname is a hostname; a line telling a person the obvious is
/// a line they learn to skip, and then they skip the one that mattered.
fn shape_of(label: &str) -> &'static str {
    match label {
        "Address" => "with the prefix, e.g. 192.168.1.50/24",
        "Gateway" => "the router, and no prefix, e.g. 192.168.1.1",
        "Timezone" => "a number above, or any IANA name, e.g. America/Chicago",
        "Auto-run" => "an application you WILL install, or blank",
        NAMESERVERS => "up to three IPv4 addresses, in the order asked, e.g. 192.168.1.1, 1.1.1.1",
        _ => "",
    }
}

/// The check each field's answer goes through.
fn checker(label: &str) -> fn(&str) -> Result<(), String> {
    match label {
        "Hostname" => field::hostname,
        "Address" => field::ipv4_cidr,
        "Gateway" => field::ipv4,
        // **The one check in this tool that a grammar cannot make.**
        // `America/Chigago` is a well-formed identifier for a file no image
        // carries, and the machine keeps UTC without saying so — see
        // `crate::zones`.
        "Timezone" => zones::check,
        NAMESERVERS => field::nameservers,
        _ => field::anything,
    }
}

/// Where the control plane listens, **decided by what will administer this
/// machine** — `ADR-004` §6, its role-defaults section.
///
/// `127.0.0.1:7777` is `rivet_config`'s default and stays the schema's, because
/// a machine that has LOST its document must not answer the network on the
/// strength of a missing file. It is the wrong thing to write into a document
/// for a machine somebody intends to administer: on 2026-09-07 it produced a
/// headless board that answered nothing, and the only recovery was rebuilding
/// the card.
///
/// **A fleet `server` or `member` is administered from elsewhere by
/// definition**, so it binds every interface. A `stand-alone` machine is not,
/// and the tool has no reason to believe its card will carry a TLS identity —
/// without one `cmd/rivet-mcpd` REFUSES a non-loopback address by name
/// (`ADR-035` §7), which would leave that machine with no control plane at
/// all rather than one reachable only from itself. So it keeps loopback, and
/// [`compose`] says so on the screen.
fn listen_for(fleet: &str) -> &'static str {
    match fleet {
        "server" | "member" => REACHABLE_LISTEN,
        _ => rivet_config::control_plane::DEFAULT_LISTEN,
    }
}

/// Every interface, which is what a machine somebody else administers must
/// answer on.
const REACHABLE_LISTEN: &str = "0.0.0.0:7777";

/// Turn the answers into a document, and let the real parser rule on it.
///
/// # Errors
///
/// Whatever `rivet-config` says, unchanged. **Its words and not a translation
/// of them** — two spellings of one refusal is `ADR-011` broken for the sake of
/// a nicer sentence.
/// The application that is the machine's front door — `ADR-046`.
const PROXY_SERVICE: &str = "rivet-proxyd";

/// The display server a kiosk's auto-run application draws through. A kiosk
/// machine needs it enabled or the display-taking application starts, finds no
/// display server holding DRM master, and fails — the black screen dev-01
/// booted to on 2026-09-22, whose launcher crash-looped because this was not in
/// the card's `[services]`. The GPU launcher (and any graphical kiosk app)
/// draws through it; a headless machine has no display and does not carry it.
const DISPLAY_SERVICE: &str = "rivet-displayd";

/// The name on this machine's fleet certificate, or empty if there is no
/// authority to read one out of.
///
/// **Composed the way `ca server` composes it**, from the suffixes in the
/// authority's own certificate rather than from the note beside it: a
/// certificate cannot drift from itself.
fn upstream_name_for(hostname: &str) -> String {
    let Some(dir) = authority_directory() else {
        return String::new();
    };
    let Ok(certificate) = std::fs::read_to_string(dir.join("root.crt")) else {
        return String::new();
    };
    let Ok(suffixes) = ca::permitted_suffixes(&certificate) else {
        return String::new();
    };
    every_name_for(hostname, &suffixes)
        .first()
        .cloned()
        .unwrap_or_default()
}

fn compose(fields: &[Field], upstream: &str, repository_id: &str) -> Result<String, String> {
    let get = |label: &str| {
        fields
            .iter()
            .find(|f| f.label == label)
            .map_or("", |f| f.value.as_str())
    };
    let mut doc = String::from("version = 1\n");
    doc.push_str(&format!("hostname = \"{}\"\n", get("Hostname")));
    // **`timezone` is a TOP-LEVEL key and it has to be written here**, before
    // the first table header. There is no appending it afterwards: a document
    // that ends in `[services.rivet-mdnsd]` has no top level left to add to,
    // which is the trap `aura-controller`'s General page walks a person into
    // when it tells them to change the zone by appending to the document.
    //
    // A blank answer never reaches this: `zones::check` refuses it, so the menu
    // will not compose at all until the operator has said something. The guard
    // is here anyway because `compose` is also what the tests call.
    let zone = get("Timezone");
    if !zone.is_empty() {
        doc.push_str(&format!("timezone = \"{zone}\"\n"));
    }
    doc.push_str("\n[network.eth0]\n");
    let method = get("Addressing");
    doc.push_str(&format!("method = \"{method}\"\n"));
    if method == "static" {
        doc.push_str(&format!("address = \"{}\"\n", get("Address")));
        let gw = get("Gateway");
        if !gw.is_empty() {
            doc.push_str(&format!("gateway = \"{gw}\"\n"));
        }

        // **`[network.resolver]` for a static address, and never for dhcp.**
        // A static machine has no lease to learn a resolver from, so without
        // this table every lookup it makes fails inside the application that
        // made it. Under dhcp the table is left out, so the lease fills it.
        // Refused here rather than composed empty: `rivet_config` would accept
        // an empty list, and that is the defect this table exists to close.
        let asked = get(NAMESERVERS);
        field::nameservers(asked).map_err(|why| format!("{NAMESERVERS}: {why}"))?;
        let list: Vec<String> = field::nameserver_list(asked)
            .iter()
            .map(|one| format!("\"{one}\""))
            .collect();
        doc.push_str(&format!(
            "\n[network.{}]\nnameservers = [{}]\n",
            rivet_config::resolver::Resolver::TABLE,
            list.join(", ")
        ));
    }

    let fleet = get("Fleet");

    // **The table is ALWAYS written, whatever the address in it.**
    //
    // Not only so a fleet server is reachable -- `listen_for` has that
    // argument. A `[control-plane]` table that is absent cannot be edited: the
    // repair for a machine listening in the wrong place is `apply edit`, which
    // takes a path plus `--expect`/`--replace` or `--append`, and appending a
    // whole table to a document that ends in one puts the new keys inside the
    // last table rather than at the top level. A key that is present can be
    // changed; a table that is missing has to be composed by hand, on a machine
    // whose control plane is the thing being repaired.
    doc.push_str(&format!("\n[control-plane]\nlisten = \"{}\"\n", listen_for(fleet)));

    // **`[kiosk]` for a machine that drives a screen**, empty if the operator
    // has not decided which application takes it. An empty table is a real
    // answer to `rivet_config` -- `Document::auto_run` covers *a table that
    // named nothing* -- and it leaves a key-shaped hole an `apply edit` can
    // fill later, for the reason `[control-plane]` above is unconditional.
    let auto_run = get("Auto-run");
    if get("Mode") == "kiosk" {
        doc.push_str("\n[kiosk]\n");
        if !auto_run.is_empty() {
            doc.push_str(&format!("auto-run = \"{auto_run}\"\n"));
        }
    }

    let services = services_for(fleet, get("Mode"), auto_run);

    // **`[applications.rivet-proxyd] upstream-name`, wherever the proxy is
    // enabled** — `SPEC-05` §5, and enabling the service without it is worse
    // than not enabling it.
    //
    // With no setting `cmd/rivet-proxyd` falls back to the first DNS name on
    // its own certificate (`ADR-060`), and refuses to start when that names no
    // host. Composing the value here keeps the document saying which name the
    // front door verifies, rather than leaving it to whichever certificate the
    // machine happens to hold — the state `SPEC-05` §10 records being repaired
    // by hand was a front door that started five times and failed.
    //
    // The value is the name on the fleet certificate, and this tool has it:
    // `<hostname>.<suffix>` for the first suffix the authority permits, which is
    // the subject `ca server` and `mint_for_host` both compose.
    if services.iter().any(|s| s == PROXY_SERVICE) {
        if upstream.is_empty() {
            return Err(format!(
                "{PROXY_SERVICE} is enabled and its `upstream-name` could not be composed, so \
                 this document would produce a machine whose front door starts five times and \
                 then fails. The value is <hostname>.<suffix> for a suffix the authority permits, \
                 and it is read out of ~/.rivet/ca/root.crt — mint the authority first with \
                 `rivet-provision ca <suffix> <label> <dir>`"
            ));
        }
        doc.push_str(&format!(
            "\n[applications.{PROXY_SERVICE}]\nupstream-name = \"{upstream}\"\n"
        ));
    }

    for service in services {
        doc.push_str(&format!("\n[services.{service}]\nenabled = true\n"));
    }

    // **`[repos.fleet]` on every fleet SERVER, and on nothing else** —
    // `.rulings/` R-184 D3, `ADR-052` §9 step 4. A server's card carries the
    // fleet repository's text (R-182) and its objects follow after first boot;
    // what lets `cargo rivet fleet push` put that text on the machine is a git
    // repository the machine serves, and `rivet-sysd` creates one on every
    // boot from exactly this declaration (`cmd/rivet-sysd/src/repos.rs`). A
    // member or a stand-alone machine serves no fleet repository, so it
    // declares none. The id is MINTED by the caller rather than here — see
    // [`mint_repository_id`] — so the same answers compose the same bytes.
    if fleet == "server" {
        if rivet_config::repos::RepoId::new(repository_id).is_none() {
            return Err(format!(
                "the repository identifier {repository_id:?} is not one this machine would \
                 accept: {}",
                rivet_config::repos::RepoId::RULE
            ));
        }
        doc.push_str(&format!(
            "\n[repos.{}]\nid = \"{repository_id}\"\n",
            rivetos_fleet_repo::SERVED_NAME
        ));
    }

    match rivet_config::Document::parse(&doc) {
        Ok(_) => Ok(doc),
        Err(why) => Err(format!("{why:?}")),
    }
}

/// **A fresh `[repos.fleet]` identifier** — `rivet_core::identifier`'s
/// grammar, sixteen lowercase hex characters, which is what
/// `rivet_config::repos::RepoId` accepts.
///
/// **Minted, never chosen**, as that type's own documentation requires, and
/// `ADR-025` §4 asks only that it be stable and opaque: it is not a secret,
/// and knowing one is not authority. What it must not do is collide.
///
/// **The entropy is the standard library's, and no randomness crate is added
/// for it.** A `std::collections::hash_map::RandomState` is keyed from the
/// operating system's random source when a thread first builds one, so
/// hashing the clock and the process id under a fresh one yields 64 bits no
/// other run reproduces. That is `cmd/rivet-fleet`'s `mint_identifier`
/// argument (three independent sources, adequate for an identifier and not
/// for a key), with a better first source than a heap address.
fn mint_repository_id() -> String {
    use std::hash::{BuildHasher as _, Hasher as _};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    if let Ok(since) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        hasher.write_u128(since.as_nanos());
    }
    hasher.write_u32(std::process::id());
    rivet_core::identifier::from_entropy(&hasher.finish().to_be_bytes())
}

/// **What a role turns on, sorted**, so that two runs of the same answers
/// produce the same bytes.
///
/// * **A fleet server runs three things, and none of them is on by default.**
///   `rivet-fleetd` is the fleet itself; `rivet-proxyd` is the front door every
///   application is reached through; `rivet-mdnsd` is what makes the name in
///   that machine's certificate resolve at all. A server provisioned without
///   the third has a certificate for a name nothing publishes, and the failure
///   shows up in a browser as a name it cannot find rather than as a service
///   that is off (ruled 2026-09-01).
/// * **A fleet member announces itself too, AND runs the front door** —
///   `ADR-050` §5's role table: `member + headless` enables `rivet-proxyd`,
///   the same way a server does, so a member reached at its own address
///   authenticates a caller rather than serving nothing on 443. It is reached
///   by name by the server that manages it, and by a person typing that name.
///   The proxy is part of the image (`ADR-060`), so enabling it is all a
///   member's card needs; nothing is carried for it.
/// * **The application that takes the display is enabled by name.** An
///   `auto-run` naming an application that is installed and NOT enabled is the
///   same black screen as one naming an application that is absent — the
///   supervisor starts nothing the document has not enabled (`ADR-015` §3),
///   so the two halves belong in one place.
fn services_for(fleet: &str, mode: &str, auto_run: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if fleet == "server" {
        out.push("rivet-fleetd".to_owned());
    }
    if fleet == "server" || fleet == "member" {
        out.push(PROXY_SERVICE.to_owned());
        out.push("rivet-mdnsd".to_owned());
    }
    if mode == "kiosk" {
        // The display server first: the auto-run application draws through it,
        // and without it the application starts, finds no DRM master and fails.
        out.push(DISPLAY_SERVICE.to_owned());
        if !auto_run.is_empty() && !out.iter().any(|s| s == auto_run) {
            out.push(auto_run.to_owned());
        }
    }
    out.sort();
    out
}

/// The fleet repository this interview lists application versions from —
/// `.rulings/` R-178 P3.
///
/// **`RIVET_FLEET_REPOSITORY`, set by the orchestrator, wins.** `cargo rivet
/// provision` resolves the repository root once for its own document commit
/// (R-178 P4) and passes it down as this environment variable — the same name
/// `tools/container/build-card.sh` already uses for the identical directory
/// (`ADR-011`) — so the interview and the orchestrator never disagree about
/// which repository a run means.
///
/// **Run on its own, with no orchestrator and no variable, this falls back to
/// the same default every other fleet tool uses** —
/// `rivetos_fleet_repo::default_root()`, `~/.rivet/fleet`.
fn fleet_repository_root() -> Option<std::path::PathBuf> {
    if let Some(value) = std::env::var_os("RIVET_FLEET_REPOSITORY") {
        let path = std::path::PathBuf::from(value);
        if !path.as_os_str().is_empty() {
            return Some(path);
        }
    }
    rivetos_fleet_repo::default_root()
}

/// **P3: the interview picks a version for every product the role carries.**
///
/// `doc` is already a composed, schema-valid document with no `[carry]` table
/// yet. For each product [`rivet_config::Document::carried_services`] names —
/// `rivet-fleetd` for a server, nothing for a member (its proxy is in the image,
/// `ADR-060`), a kiosk's auto-run application — this lists what `root` has published
/// (`crate::versions::list`, newest first), lets the operator pick
/// ([`choose_carried_version`], newest preselected — R-176 Q6), and appends
/// `[carry.<product>] version = "…"` to the document.
///
/// **A required product with no published version is a refusal, before
/// anything is written** — naming the product and `cargo rivet fleet publish`
/// as the fix, on `cmd/rivet-image/src/fleet_payloads.rs::resolve`'s own
/// footing for the identical gap.
///
/// A document that carries nothing (a stand-alone machine) is returned
/// unchanged — nothing here asks a question nobody needs answered.
///
/// # Errors
///
/// `doc` does not parse (should not happen — `compose` already validated it,
/// this is defence in depth); `root` is `None` and a product must be carried;
/// `root`'s manifest cannot be read; a required product has no published
/// `application` version; or the operator's answer names something that is
/// not one of the versions offered.
fn resolve_carry(doc: String, root: Option<&std::path::Path>) -> Result<String, String> {
    let parsed = rivet_config::Document::parse(&doc)
        .map_err(|e| format!("the document just composed does not parse: {e:?}"))?;
    let products: Vec<String> =
        parsed.carried_services().iter().map(|s| s.name().as_str().to_owned()).collect();
    if products.is_empty() {
        return Ok(doc);
    }

    let Some(root) = root else {
        return Err(format!(
            "NOTHING WAS WRITTEN. this machine must carry {} (ADR-050 §5.1) and no fleet \
             repository could be found to select a version from. Set RIVET_FLEET_REPOSITORY, or \
             create one at the default location with `cargo rivet fleet init`, then run again.",
            products.join(", ")
        ));
    };

    let mut doc = doc;
    for product in &products {
        let versions = versions::list(root, product)?;
        if versions.is_empty() {
            return Err(format!(
                "NOTHING WAS WRITTEN. {product} must be carried (ADR-050 §5.1) and the fleet \
                 repository at {} holds no published `application` artifact of that name. \
                 `cargo rivet fleet publish` is how one gets there — publish it, then run this \
                 again.",
                root.display()
            ));
        }
        let chosen = choose_carried_version(product, &versions)?;
        doc.push_str(&format!("\n[carry.{product}]\nversion = \"{chosen}\"\n"));
    }

    // Defence in depth: the tables just appended must still be a document a
    // board would accept.
    rivet_config::Document::parse(&doc)
        .map_err(|e| format!("the document with its carry pins does not parse: {e:?}"))?;
    Ok(doc)
}

/// **The version menu for one carried product** — R-176 Q6's "newest
/// preselected", on `cmd/cargo-rivet/src/write_image.rs`'s own menu convention: a
/// numbered list, the newest first and marked, a bare Return taking it.
/// Prints the menu, reads one line from stdin, and hands it to
/// [`carried_version_from_answer`] — split out so the decision itself is
/// provable with no stdin at all.
///
/// `versions` is never empty — the caller refuses before this is reached.
///
/// # Errors
///
/// Input closed (EOF) while a choice was being read, [`std::io::stdin`]
/// itself failing, or [`carried_version_from_answer`]'s own refusal.
fn choose_carried_version(product: &str, versions: &[String]) -> Result<String, String> {
    // **One candidate is not a question** — `cmd/cargo-rivet/src/write_image.rs`'s
    // own `choose_with`, restated: asking it would train an operator to press
    // Return without reading, and there is nothing here for stdin to answer
    // anyway.
    if let [only] = versions {
        println!("  {product}: {only}");
        return Ok(only.clone());
    }
    println!("\n  {product} — published versions:");
    for (i, v) in versions.iter().enumerate() {
        let marker = if i == 0 { " (newest)" } else { "" };
        println!("    {})  {v}{marker}", i + 1);
    }
    println!("    x) exit — leave without choosing");
    print!("  carry [1], or x: ");
    let _ = std::io::stdout().flush();

    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        Ok(0) => {
            return Err(format!(
                "NOTHING WAS WRITTEN. input closed while choosing which published version of \
                 {product} to carry."
            ));
        }
        Ok(_) => {}
        Err(e) => return Err(format!("cannot read a choice: {e}")),
    }
    carried_version_from_answer(product, versions, line.trim())
}

/// [`choose_carried_version`]'s decision, pure: given the line already read
/// from stdin, which of `versions` did the operator choose?
///
/// **Blank is the newest** (R-176 Q6's "newest preselected", `versions[0]`
/// — the caller already sorted it that way). A number picks by position, one-
/// based, on every other numbered menu's own convention. The literal version
/// string also works — a person who was just shown `0.1.86` and typed it back
/// should not be told off for knowing the answer, the same allowance the
/// interview's own field menus make.
///
/// **`x`/`X` refuses rather than picks** — `cmd/cargo-rivet/src/write_image.rs`'s
/// `choose_with` convention, restated here for the one caller of this menu
/// that is not that binary: `resolve_carry`'s `?` turns this into the exact
/// refusal an out-of-range number already gives, which the interview's own
/// `w` handler already prints and loops on (`Err(why) => println!("  {why}")`)
/// — a document composed of several products asks this once per product, and
/// there is no single card write to roll back yet for `x` to leave clean of,
/// only the answer this one question would have added.
///
/// # Errors
///
/// `x`/`X`, a number out of range, or text that is neither empty, `x`/`X`, a
/// number, nor one of `versions` verbatim.
fn carried_version_from_answer(product: &str, versions: &[String], answer: &str) -> Result<String, String> {
    if answer.is_empty() {
        return Ok(versions[0].clone());
    }
    if answer.eq_ignore_ascii_case("x") {
        return Err(format!(
            "exited — no version of {product} was chosen, and nothing was written. Answer `w` \
             again once you have decided."
        ));
    }
    if let Ok(n) = answer.parse::<usize>() {
        return match n.checked_sub(1).and_then(|i| versions.get(i)) {
            Some(v) => Ok(v.clone()),
            None => Err(format!("{n} is not between 1 and {}", versions.len())),
        };
    }
    if versions.iter().any(|v| v == answer) {
        return Ok(answer.to_owned());
    }
    Err(format!(
        "{answer:?} is not one of the published versions of {product}: {}",
        versions.join(", ")
    ))
}

fn main() -> std::process::ExitCode {
    // **`ca <suffix> <name> <dir>`** — mint the authority a browser will trust.
    //
    // Checked BEFORE the argument parser, because Ignition's surface does not
    // know this verb and clap refuses an argument it was not told about.
    //
    // A verb rather than a menu item because it is done ONCE per fleet, and
    // because the thing it writes is the most valuable secret this project
    // has: whoever holds `root.key` can vouch for anything under the suffix.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("ca") {
        if args.get(1).map(String::as_str) == Some("server") {
            return mint_server(&args[2..]);
        }
        if args.get(1).map(String::as_str) == Some("backup") {
            return ca_backup(&args[2..]);
        }
        if args.get(1).map(String::as_str) == Some("restore") {
            return ca_restore(&args[2..]);
        }
        return mint_ca(&args[1..]);
    }
    // **`stage`**, the staging bundle for a document that already exists —
    // matched here for the reason `ca` is: the argument parser does not know it.
    if args.first().map(String::as_str) == Some("stage") {
        return stage_document::run(&args[1..]);
    }

    // **The version carries the COMMIT as well as the number** — `build.rs`,
    // and `.rulings/` R-191 D3. `cargo rivet provision` runs this binary as
    // the interview and refuses the pair unless the two builds match, so what
    // `--version` prints here has to be comparable with what `cargo-rivet
    // --version` prints: `<version> (<describe>)`, one spelling, both tools.
    // Between image bumps a workstation builds both from many commits, all of
    // them the same version, so the number alone would let a mismatched pair
    // through.
    let cli = ignition::cli::Cli::new("rivet-provision")
        .version(concat!(env!("CARGO_PKG_VERSION"), " (", env!("RIVET_BUILD_DESCRIBE"), ")"))
        .about("Compose and validate a Rivet provisioning document.");
    // **`after_help` is where the two hand-matched verbs are declared.** They
    // are intercepted above, so `clap` has never heard of them and `--help`
    // showed a tool with no subcommands at all — which is exactly where a
    // person looks for `--no-install` and does not find it.
    let new_fleet = args.iter().any(|a| a == NEW_FLEET);
    let parsed = std::env::args_os().filter(|a| a.to_str() != Some(NEW_FLEET));
    let matches = cli.command().after_help(CA_HELP).get_matches_from(parsed);
    match cli.dispatch(&matches, &ignition::update::UpdateConfig::default()) {
        Ok(ignition::cli::Outcome::Handled) => return std::process::ExitCode::SUCCESS,
        Ok(ignition::cli::Outcome::Passthrough) => {}
        Err(why) => {
            eprintln!("rivet-provision: {why}");
            return std::process::ExitCode::FAILURE;
        }
    }

    // **The authority is made HERE, on first run — once the operator has said
    // this is a new fleet.** The menu mints rather than sending a person to a
    // command-line verb, and `ADR-063` §8 puts one question in front of it:
    // a workstation joining a fleet that already has an authority must not
    // mint a second one. `ca` remains as a verb for scripts.
    match ensure_authority(new_fleet) {
        Ok(Guarded::Present | Guarded::Minted | Guarded::FleetRoot) => {}
        Ok(Guarded::Joining | Guarded::Exited) => return std::process::ExitCode::FAILURE,
        Ok(Guarded::Refused(why)) => {
            eprintln!("rivet-provision: {why}");
            return std::process::ExitCode::FAILURE;
        }
        // A failed mint after the operator chose a new fleet is reported and
        // not fatal: the menu still composes, and staging refuses by name later.
        Err(why) => println!("  {why}"),
    }

    // **The fleet repository this run selects carried application versions
    // from, resolved once, HERE** — `.rulings/` R-178 P3/P4.
    // `fleet_repository_root` takes `RIVET_FLEET_REPOSITORY` when the
    // orchestrator set it, and falls back to the same default every other
    // fleet tool uses otherwise. Unlike before R-178, nothing here commits
    // to it — `w` composes and saves LOCALLY only; `cargo rivet provision`
    // commits the document (and enrolls the register) itself, once the card
    // it describes has actually been written and verified (R-176 Q10).
    let fleet_repo_root = fleet_repository_root();

    let mut fields = questions();

    // **Offer to carry on.** Asked for after the first run: work in progress
    // should survive somebody walking away from it.
    if let Some(path) = offer_to_resume() {
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                save::apply(&text, &mut fields, checker);
                identity::follow_hostname(&mut fields, rivet_root().as_deref());
                // A saved Nameservers answer is kept; one saved before this
                // field existed, or never changed, follows the saved gateway.
                follow_gateway(&mut fields);
                println!("  resumed {}", path.display());
            }
            Err(why) => println!("  {}: {why}", path.display()),
        }
    }

    // **The timezone starts REFUSED, and that is how the menu asks for it.**
    //
    // Ruled 2026-09-07 (.rulings/ R-261): the timezone is set for his board, and
    // every later provisioning asks for it as a setting. A field with a
    // plausible default would be answered by silence, and silence here is a
    // machine that keeps UTC without saying so — so the field begins invalid,
    // `w` refuses while any field is, and the screen carries `fix 7` with the
    // reason beside it until somebody has answered.
    //
    // Every other field is left alone: `Address` and `Gateway` are empty under
    // `dhcp` and are correct that way, so validating the whole form here would
    // invent two refusals for answers nobody owes.
    for f in &mut fields {
        if f.label == "Timezone" {
            let typed = f.value.clone();
            f.set(&typed, checker(f.label));
        }
    }

    // What is on disk, so "unsaved" is a fact rather than a guess.
    let mut saved_as = save::render(&fields);
    // **Colour only when somebody is looking.** A redirected document full of
    // escape sequences is a file nothing can read.
    let colour = Colour(std::io::IsTerminal::is_terminal(&std::io::stdout()));

    loop {
        // **A field that is not asked is not on the screen**, and its number
        // is unchanged either way: `asked` hides only `Nameservers`, which is
        // last.
        let rows: Vec<Row<'_>> = fields
            .iter()
            .enumerate()
            .filter(|(_, f)| asked(&fields, f.label))
            .map(|(i, f)| Row {
                number: i + 1,
                label: f.label,
                value: if f.value.is_empty() { "—" } else { &f.value },
                state: f.state(),
            })
            .collect();
        let edit = format!("1-{} edit", rows.len());
        // A field that is not asked cannot block `w`: it is not composed.
        let bad = fields
            .iter()
            .position(|f| f.error.is_some() && asked(&fields, f.label));
        // **The reason, not just the number.** This said `fix 3` and stopped —
        // so a person who typed `192.168.50.10` was told something was wrong and
        // not what, while `field::ipv4_cidr` had already produced the exact
        // sentence they needed (*"needs a prefix, e.g. /24"*) and it was
        // discarded one line later. The screen's brevity rule is about prose,
        // and a validator's own short reason is the opposite of prose: it is the
        // thing being scanned for.
        let blocked = bad.map(|i| {
            fields[i].error.as_ref().map_or_else(
                || format!("fix {}", i + 1),
                |why| format!("fix {}: {why}", i + 1),
            )
        });
        print!(
            "\n{}",
            screen::render(
                &rows,
                &[edit.as_str(), "s save", "w write", "? explain", "q quit"],
                blocked.as_deref(),
                colour,
            )
        );
        print!("  > ");
        let _ = std::io::stdout().flush();

        let mut answer = String::new();
        match std::io::stdin().read_line(&mut answer) {
            // See the edit branch: a closed stdin is a quit.
            Ok(0) => return std::process::ExitCode::SUCCESS,
            Ok(_) => {}
            Err(_) => return std::process::ExitCode::FAILURE,
        }
        let answer = answer.trim();

        match answer {
            "q" => {
                // **Unsaved work is not thrown away silently.**
                if save::render(&fields) != saved_as {
                    print!("  unsaved. save? [Y/n] ");
                    let _ = std::io::stdout().flush();
                    let mut yes = String::new();
                    if std::io::stdin().read_line(&mut yes).is_ok()
                        && !yes.trim().eq_ignore_ascii_case("n")
                    {
                        let _ = write_answers(&fields);
                    }
                }
                return std::process::ExitCode::SUCCESS;
            }
            "s" => match write_answers(&fields) {
                Ok(path) => {
                    println!("  saved {}", path.display());
                    saved_as = save::render(&fields);
                }
                Err(why) => println!("  {why}"),
            },
            "w" => {
                if let Some(i) = bad {
                    println!("  {} is not valid", fields[i].label);
                    continue;
                }
                // **R-189 D2, before a question is asked or a file written.**
                if !identity_allowed(&fields) {
                    continue;
                }
                let upstream = upstream_name_for(&value_of(&fields, "Hostname"));
                // **R-178 P3, before anything else about this document is
                // decided.** A required product with no published version is
                // refused right here — nothing is minted, printed or written
                // for a card that could not carry what its own role needs.
                // **A fresh identifier per `w`**: nothing has been provisioned
                // until a card is written, so a second `w` after a refusal
                // composing a different id loses nothing (`compose` uses it
                // only for a server).
                let composed = compose(&fields, &upstream, &mint_repository_id())
                    .and_then(|doc| resolve_carry(doc, fleet_repo_root.as_deref()));
                match composed {
                    Ok(doc) => {
                        // **A fleet server's certificate is minted HERE**,
                        // because this is the first moment its hostname exists.
                        // A browser validates the name it asked for, so the
                        // certificate could not have been made any earlier.
                        if value_of(&fields, "Fleet") == "server" {
                            match mint_for_host(value_of(&fields, "Hostname")) {
                                Ok(path) => println!("  certificate {}", path.display()),
                                Err(why) => println!("  no certificate: {why}"),
                            }
                        }
                        println!("\n{doc}");
                        print!("{}", what_the_role_decided(&fields, orchestrated()));
                        // **`w` WRITES.** Until 2026-09-07 it minted the
                        // certificate, printed this document and exited
                        // successfully, having written nothing — and a person
                        // who typed a key labelled `write` and watched output
                        // scroll past reasonably believed a document existed.
                        // `SPEC-05` §4.
                        match write_document(&fields, &doc) {
                            Ok(Some(path)) => {
                                println!("  written {}", path.display());
                                // **And then read back off the disk and given
                                // to the board's own parser** — `SPEC-05` §4.
                                // `compose` already parsed the string in
                                // memory; this asks the different question,
                                // which is whether the FILE is a document.
                                if !check_written(&path) {
                                    return std::process::ExitCode::FAILURE;
                                }
                                // **R-189 D2: a machine's own identity, minted
                                // here** — after the document is written, so a
                                // declined overwrite mints nothing, and before
                                // staging, which reads its pin and peers.
                                if !mint_identity_if_absent(&fields) {
                                    return std::process::ExitCode::FAILURE;
                                }
                                // **R-178 P4/Q10: recording moved.** Until
                                // 2026-09-13 this committed the document into
                                // the fleet repository right here, before a
                                // card existed at all. `cargo rivet
                                // provision` now does that — and enrolls the
                                // register — itself, and ONLY once
                                // `write-image` has actually written and
                                // verified a card. This process composes and
                                // saves LOCALLY, and nothing more; see the
                                // module's own History for why the old
                                // ordering was wrong.
                                //
                                // **And the material the card must carry FOR
                                // this machine's applications** — `ADR-049`,
                                // and it is written beside the document because
                                // the two are one card.
                                if !write_staging(&fields, &path) {
                                    return std::process::ExitCode::FAILURE;
                                }
                                // **This card's own control-plane pin, best
                                // effort** — `ADR-049` §8.1: the orchestrator
                                // derives the register identifier it enrolls
                                // FROM this, and it has no other way to reach
                                // it than through the breadcrumb below, since
                                // "Identity" is a workstation fact this
                                // interview never writes into the document
                                // itself. Read only when the operator gave a
                                // directory; a missing or unreadable one is
                                // not fatal here — `cargo rivet provision`'s
                                // own `--identity`/`--pin` is the fallback,
                                // named in its own refusal if enrollment turns
                                // out to need one and none was found.
                                let identity_value = value_of(&fields, "Identity");
                                let pin_hex = (!identity_value.trim().is_empty())
                                    .then(|| stage::device_pin(std::path::Path::new(identity_value.trim())))
                                    .and_then(Result::ok)
                                    .map(|pin| rivetos_tls::encode_hex(&pin));
                                // **The hand-off `cargo rivet provision`'s
                                // orchestrator reads** — plan §5.2 item B,
                                // `.rulings/` R-162, Wave 3b. Written last,
                                // only once every step above has already
                                // succeeded, so its presence means the whole
                                // of this arm ran rather than merely that `w`
                                // was pressed.
                                let identity_dir_value =
                                    (!identity_value.trim().is_empty()).then(|| identity_value.trim());
                                write_breadcrumb(&path, pin_hex.as_deref(), identity_dir_value);
                            }
                            // Declining to overwrite is an answer, not a fault.
                            // The menu stays open so the hostname can be
                            // changed and `w` pressed again.
                            Ok(None) => continue,
                            Err(why) => {
                                println!("  NOT written: {why}");
                                return std::process::ExitCode::FAILURE;
                            }
                        }
                        return std::process::ExitCode::SUCCESS;
                    }
                    // **The schema's own words.** This is the whole reason the
                    // tool links `rivet-config` rather than knowing the rules.
                    Err(why) => println!("  refused: {why}"),
                }
            }
            "?" => {
                for (i, f) in fields.iter().enumerate() {
                    if asked(&fields, f.label) {
                        println!("  {}  {}  {}", i + 1, f.label, f.explain);
                    }
                }
            }
            n => {
                let Ok(index) = n.parse::<usize>() else {
                    continue;
                };
                let Some(label) = index.checked_sub(1).and_then(|i| fields.get(i)).map(|f| f.label)
                else {
                    continue;
                };
                if !asked(&fields, label) {
                    println!("  {index} is asked only for a static address; dhcp's lease names the nameservers");
                    continue;
                }
                let Some(f) = index.checked_sub(1).and_then(|i| fields.get_mut(i)) else {
                    continue;
                };

                // **A choice is picked by NUMBER**, not typed. Ruled on the
                // first run (.rulings/ R-346): the dhcp-or-static choice
                // should be a numbered menu, so that nobody has to type out
                // the word. Typing a word you were just shown is
                // transcription, and this menu is numbers everywhere else.
                //
                // **The word still works**, because somebody who typed
                // `static` should not be told off for knowing the answer.
                //
                // **A text field may offer numbers too**, and `Timezone` is
                // why: six hundred legal answers cannot be a `Kind::Choice`,
                // but the eleven a person is most likely to want can still be
                // pressed rather than typed. Anything not on the list is typed
                // in full and checked the same way.
                let offered = if let Kind::Choice(of) = &f.kind {
                    for (i, o) in of.iter().enumerate() {
                        println!("  {}  {o}", i + 1);
                    }
                    of.clone()
                } else {
                    let suggestions = offered_for(f.label);
                    for (i, o) in suggestions.iter().enumerate() {
                        println!("  {}  {o}", i + 1);
                    }
                    if f.label == "Timezone" {
                        println!("  checked against {}", zones::source());
                    }
                    suggestions
                };

                // **The shape of the answer, before the first attempt.** The
                // form was only ever in the `?` text, and nobody presses `?`
                // before their first try — so the first thing a person learned
                // about the prefix was being refused for omitting it.
                let shape = shape_of(f.label);
                if !shape.is_empty() {
                    println!("  {shape}");
                }
                // **The default is shown where a bare Return takes it** — the
                // `ask` prompts' `label [default]` shape, on the one field whose
                // empty answer means its default.
                if f.label == NAMESERVERS && !f.default.is_empty() {
                    print!("  {} [{}] > ", f.label, f.default);
                } else {
                    print!("  {} > ", f.label);
                }
                let _ = std::io::stdout().flush();
                let mut given = String::new();
                match std::io::stdin().read_line(&mut given) {
                    // **EOF is a quit, not a spin.** A closed stdin used to
                    // return an empty line for ever and this loop took it as an
                    // unrecognised command, so a piped run never ended.
                    Ok(0) => return std::process::ExitCode::SUCCESS,
                    Ok(_) => {}
                    Err(_) => return std::process::ExitCode::FAILURE,
                }
                let chosen = chosen_answer(f, given.trim(), &offered);
                let check = checker(f.label);
                f.set(&chosen, check);
                // **A refused list is refused at the prompt, by name.** The
                // footer names only the FIRST field that will not do, and
                // Timezone starts refused, so a refusal of the tenth field
                // would otherwise sit behind it unread.
                if f.label == NAMESERVERS
                    && let Some(why) = &f.error
                {
                    println!("  refused: {why}");
                }
                let hostname_answered = f.label == "Hostname" && f.error.is_none();
                let gateway_moved = matches!(f.label, "Address" | "Gateway") && f.error.is_none();

                // **A gateway the address already implies.** Asked for on the
                // first run (.rulings/ R-346): the gateway is predicted as a
                // default from the address. There is no reason not to — the
                // router is at `.1` on essentially every home LAN.
                //
                // Offered as a DEFAULT and not as an answer: it stays dim, so
                // it reads as a suggestion the operator has not agreed to yet.
                if f.label == "Address" && f.error.is_none() {
                    if let Some(implied) = field::implied_gateway(&chosen) {
                        if let Some(g) = fields.iter_mut().find(|g| g.label == "Gateway") {
                            if g.value.is_empty() || g.value == g.default {
                                g.default.clone_from(&implied);
                                g.value = implied;
                                g.error = None;
                            }
                        }
                    }
                }

                // **The Identity default follows the hostname** until the
                // operator types one — `.rulings/` R-189 D2.
                if hostname_answered {
                    identity::follow_hostname(&mut fields, rivet_root().as_deref());
                }
                // **And the Nameservers default follows the gateway**, whether
                // the gateway was typed or implied by the address above.
                if gateway_moved {
                    follow_gateway(&mut fields);
                }
            }
        }
    }
}

/// **Say what the ROLE put in the document, and what each line costs if the
/// rest of the card does not match it.**
///
/// The document is printed above this, so none of it is news. What is not
/// visible in the text is the *coupling*: a listener that needs an identity on
/// the card, and an `auto-run` that needs an application that will be
/// installed. Both fail on a booted board with nothing on a screen, and both
/// are cheap to state here.
///
/// **No card-building advice, on either route** — `ADR-054` §2: a card comes
/// from the fleet repository, and `cargo rivet provision` puts the identity on
/// it. Under that orchestrator ([`orchestrated`]) the identity is placed by the
/// same run moments later, so this says so in one line and nothing more.
fn what_the_role_decided(fields: &[Field], orchestrated: bool) -> String {
    let mut out = String::new();
    let fleet = value_of(fields, "Fleet");
    let listen = listen_for(&fleet);
    out.push_str(&format!("  the {fleet} role set [control-plane] listen = {listen}\n"));
    if listen != REACHABLE_LISTEN {
        out.push_str(
            "    loopback, so this machine answers only itself. A stand-alone machine\n\
             \x20   may have no identity on its card, and rivet-mcpd refuses a non-loopback\n\
             \x20   address without one — which would be no control plane rather than a\n\
             \x20   private one. Edit the key above if this machine is administered\n\
             \x20   remotely and its card carries an identity.\n",
        );
    } else if orchestrated {
        out.push_str(
            "    reachable from the network; cargo rivet provision puts the Identity\n\
             \x20   answer on the card, which rivet-mcpd needs to listen there\n",
        );
    } else {
        out.push_str(
            "    reachable from the network, which is what makes this machine\n\
             \x20   administrable at all. The CARD MUST CARRY A TLS IDENTITY: without one\n\
             \x20   rivet-mcpd refuses a non-loopback address by name and the machine has\n\
             \x20   NO control plane. The Identity answer is that identity, and\n\
             \x20   cargo rivet provision puts it on the card.\n",
        );
    }

    let auto_run = value_of(fields, "Auto-run");
    if value_of(fields, "Mode") == "kiosk" {
        if auto_run.is_empty() {
            out.push_str("  [kiosk] names no application, so nothing takes the display yet\n");
        } else if rivet_core::service::IMAGE_SERVICE_NAMES.contains(&auto_run.as_str()) {
            // **An image service is in `/system` on every card**, so there is
            // nothing to install and nothing to warn about — the same list
            // `Document::carried_services` draws its line with.
            out.push_str(&format!(
                "  [kiosk] auto-run = {auto_run}, which the system image itself carries\n"
            ));
        } else {
            out.push_str(&format!(
                "  [kiosk] auto-run = {auto_run}, and that application is enabled\n\
                 \x20   NOTHING HERE CAN CHECK THAT IT WILL BE INSTALLED. An auto-run naming\n\
                 \x20   an application the machine does not have takes the display and draws\n\
                 \x20   nothing, with no message on the screen and none in the journal that\n\
                 \x20   says so plainly. Install it before the first boot, or leave this blank.\n"
            ));
        }
    }
    out
}

/// **Whether `cargo rivet provision` is running this interview as one stage of
/// its own run** — `SPEC-05` §4. The orchestrator sets [`ORCHESTRATED`] on
/// this process; run directly, nobody does.
///
/// Neither `RIVET_AT` nor `RIVET_FLEET_REPOSITORY` answers this: a person
/// running this tool directly may set either, and the second is not always set
/// by the orchestrator.
fn orchestrated() -> bool {
    std::env::var_os(ORCHESTRATED).is_some_and(|v| !v.is_empty())
}

/// The variable [`orchestrated`] reads. `cmd/cargo-rivet/src/provision.rs`
/// spells the same name.
const ORCHESTRATED: &str = "RIVET_PROVISION_ORCHESTRATED";

/// **What to say once the staging bundle is written**, beyond its path.
///
/// Under the orchestrator: nothing — the same run writes the card next.
/// Run directly: the one command that writes a card from what `w` just saved,
/// which selects its image from the fleet repository (`ADR-054` §2).
fn after_staging(orchestrated: bool, document: &std::path::Path, identity: &str) -> Option<String> {
    if orchestrated {
        return None;
    }
    let mut line = format!("  write the card with:  cargo rivet provision --document {}", document.display());
    if !identity.trim().is_empty() {
        line.push_str(&format!(" --identity {}", identity.trim()));
    }
    Some(line)
}

/// Mint a fleet's certificate authority and its first two leaves.
///
/// **Three files and one warning.** The warning is the point: `root.key` is
/// the fleet's trust, and this is the only copy.
fn mint_ca(args: &[String]) -> std::process::ExitCode {
    // **The operator's name is a parameter and the fleet's is not the same
    // thing.** `<fleet-name>` labels the AUTHORITY — what a trust store shows.
    // `<operator-name>` is the person, and it is the string `rivet-proxyd`'s
    // index prints as *"Authenticated as: …"* and the journal records beside
    // every change they make. Optional so that scripts keep working; the
    // interactive path asks.
    // **Flags are pulled out before the positionals are counted**, so
    // `--no-install` may be typed wherever a person thinks of it. An unknown
    // `--flag` is refused by name rather than silently becoming a suffix list:
    // this verb's first positional is the one field a mint cannot take back.
    let mut into_store = true;
    let mut positional: Vec<String> = Vec::new();
    for one in args {
        match one.as_str() {
            NO_INSTALL => into_store = false,
            other if other.starts_with("--") => {
                eprintln!("rivet-provision: {other} is not a flag this verb takes");
                eprintln!("  the only one is {NO_INSTALL}");
                return std::process::ExitCode::FAILURE;
            }
            _ => positional.push(one.clone()),
        }
    }

    let (suffix, name, dir, operator) = match positional.as_slice() {
        [suffix, name, dir] => (suffix, name, dir, DEFAULT_OPERATOR.to_owned()),
        [suffix, name, dir, operator] => (suffix, name, dir, operator.clone()),
        _ => {
            eprintln!(
                "usage: rivet-provision ca <suffixes> <fleet-name> <directory> [operator-name] \
                 [{NO_INSTALL}]"
            );
            eprintln!("  <suffixes> is one or more, comma-separated. They cannot be changed");
            eprintln!("  afterwards, so name every one this fleet may ever be reached by.");
            eprintln!("  The interactive path offers `local` and `<fleet>.home.arpa`, which");
            eprintln!("  is RFC 8375's reserved namespace and can never be delegated.");
            eprintln!("  {NO_INSTALL} writes browser.pfx and does NOT touch the certificate");
            eprintln!("  store — the only way to rehearse this ceremony without leaving a");
            eprintln!("  certificate in your real Personal store.");
            eprintln!("  e.g. rivet-provision ca local home-fleet ~/.rivet/ca \"Your Name\"");
            eprintln!(
                "       rivet-provision ca local,home-fleet.home.arpa home-fleet ~/.rivet/ca"
            );
            return std::process::ExitCode::FAILURE;
        }
    };
    let suffixes = split_suffixes(suffix);

    let authority = match ca::Authority::root(name, &suffixes) {
        Ok(a) => a,
        Err(why) => {
            eprintln!("rivet-provision: {why}");
            return std::process::ExitCode::FAILURE;
        }
    };

    // **No server certificate here, and that is the correction.**
    //
    // The first version minted one for `<fleet-name>.<suffix>`, which quietly
    // turned a LOGICAL LABEL into a HOSTNAME. The maintainer caught it by asking
    // whether the fleet name was the name of the host the fleet is on or only a
    // logical name for the fleet, and said he had assumed, probably wrongly, the
    // latter (.rulings/ R-340).
    //
    // The assumption was right and the code was wrong. `name` labels the
    // AUTHORITY — it is what appears in a trust store — and a fleet server's
    // certificate has to carry the name a browser will actually ask for, which
    // is that machine's hostname. **That is not known yet**: the authority
    // exists before the machine does. `ca server` mints it once it is.
    // The suffix is recorded beside the authority; see `ensure_authority`.
    //
    // **The directory is made FIRST, and the failure is not swallowed.** This
    // write used to sit above `create_dir_all` behind a `let _ =`, so minting
    // into a directory that did not exist yet — which is every first mint —
    // silently produced an authority with no `suffix` note beside it. Verified
    // 2026-09-07 by minting into a fresh scratch directory and getting four
    // files where the tool's own summary promises five.
    if let Err(why) = std::fs::create_dir_all(dir) {
        eprintln!("rivet-provision: {dir}: {why}");
        return std::process::ExitCode::FAILURE;
    }
    let note = std::path::Path::new(dir).join("suffix");
    if let Err(why) = std::fs::write(&note, suffixes.join("\n")) {
        eprintln!("rivet-provision: {}: {why}", note.display());
        return std::process::ExitCode::FAILURE;
    }

    let parts = [
        ("root", authority.pem()),
        // **No DNS name.** A client certificate identifies a PERSON, and a
        // person does not have a hostname. The first version put
        // `operator.<suffix>` in here, which invented one -- the same mistake
        // as naming a server after a fleet, in a different costume. And naming
        // no host is what makes it the operator's (`Authority::operator`).
        ("browser", authority.operator(&operator)),
    ];

    let mut identity: Option<ca::Minted> = None;
    for (label, minted) in parts {
        let minted = match minted {
            Ok(m) => m,
            Err(why) => {
                eprintln!("rivet-provision: {label}: {why}");
                return std::process::ExitCode::FAILURE;
            }
        };
        for (ext, body) in [("crt", &minted.certificate), ("key", &minted.key)] {
            let path = std::path::Path::new(dir).join(format!("{label}.{ext}"));
            if let Err(why) = std::fs::write(&path, body) {
                eprintln!("rivet-provision: {}: {why}", path.display());
                return std::process::ExitCode::FAILURE;
            }
            println!("  {}", path.display());
        }
        if label == "browser" {
            identity = Some(minted);
        }
    }

    if let Some(minted) = identity {
        enroll(std::path::Path::new(dir), &minted, into_store);
    }

    println!();
    println!("  root.crt     DOUBLE-CLICK THIS. Install for Current User, into");
    println!("               Trusted Root Certification Authorities");
    println!("  root.key     THE FLEET'S TRUST. One copy. Back it up, keep it off the fleet");
    println!("  browser.pfx  YOUR identity. Already filed if this said so above; this copy");
    println!("               is what you carry to a second computer");
    println!();
    println!("  {name} names the AUTHORITY, not a machine — it is what a trust store shows");
    println!(
        "  constrained to {} — it cannot vouch for anything else, ever",
        suffixes
            .iter()
            .map(|s| format!(".{s}"))
            .collect::<Vec<_>>()
            .join(" and ")
    );
    println!();
    println!("  next: rivet-provision ca server <hostname> {dir}");
    println!("        once the fleet server has a hostname, which is what a browser asks for");
    std::process::ExitCode::SUCCESS
}

/// **`ca backup <directory>`** — copy the authority somewhere the operator
/// chose.
///
/// `<directory>` is REQUIRED and there is no default: a default location for
/// storing secret keys is an absurd statement, ruled by the maintainer
/// 2026-09-09 the day before a brand-new fleet's authority was minted.
/// `cargo rivet backup`'s `--into` and Fleet Monitor's `backups.folder` are
/// the same rule for the same reason — a tool that invents a destination is
/// one whose output is somewhere the operator did not choose, and they find
/// out when they need it. `SPEC-05` §4.
fn ca_backup(args: &[String]) -> std::process::ExitCode {
    let mut replace = false;
    let mut positional: Vec<String> = Vec::new();
    for one in args {
        match one.as_str() {
            REPLACE => replace = true,
            other if other.starts_with("--") => {
                eprintln!("rivet-provision: {other} is not a flag this verb takes");
                eprintln!("  the only one is {REPLACE}");
                return std::process::ExitCode::FAILURE;
            }
            _ => positional.push(one.clone()),
        }
    }
    let [dest] = positional.as_slice() else {
        eprintln!("usage: rivet-provision ca backup <directory> [{REPLACE}]");
        eprintln!("  <directory> is REQUIRED. There is no default location for storing");
        eprintln!("  the fleet's whole trust — you choose where it goes.");
        eprintln!("  {REPLACE} overwrites a backup already at <directory>; without it, an");
        eprintln!("  existing backup is left alone and this refuses.");
        eprintln!("  e.g. rivet-provision ca backup D:\\Backups\\rivet-ca");
        return std::process::ExitCode::FAILURE;
    };

    let Some(source) = authority_directory() else {
        eprintln!("rivet-provision: no home directory, so there is no authority to back up");
        return std::process::ExitCode::FAILURE;
    };
    if !source.join("root.crt").exists() {
        eprintln!(
            "rivet-provision: {} holds no authority yet — nothing to back up",
            source.display()
        );
        return std::process::ExitCode::FAILURE;
    }

    match backup::copy_all(&source, std::path::Path::new(dest), replace) {
        Ok(copied) => {
            println!("  backed up {} to {dest}", source.display());
            print_copied(&copied);
            println!();
            println!(
                "  {} file(s) copied. This is the fleet's whole trust — keep this copy",
                copied.len()
            );
            println!("  somewhere {dest} itself is backed up, and off the fleet.");
            std::process::ExitCode::SUCCESS
        }
        Err(backup::Error::Empty(dir)) => {
            eprintln!("rivet-provision: {} holds nothing to copy", dir.display());
            std::process::ExitCode::FAILURE
        }
        Err(backup::Error::WouldOverwrite(existing)) => {
            eprintln!("rivet-provision: {dest} already holds a backup. NOT overwritten.");
            print_existing(&existing);
            eprintln!("  pass {REPLACE} to replace it, or choose a different directory.");
            std::process::ExitCode::FAILURE
        }
        Err(backup::Error::Io(why)) => {
            eprintln!("rivet-provision: {why}");
            std::process::ExitCode::FAILURE
        }
        // **A real outcome, not a defensive stub.** `<directory>` naming the
        // authority itself, or a location inside it, is a plausible typo —
        // and before B05's fix in `rivet-filecopy`, this reached
        // `std::fs::copy(file, file)` unguarded and truncated the one copy
        // of the fleet's whole trust while reporting success.
        Err(backup::Error::SameFile(paths)) => {
            eprintln!("rivet-provision: {dest} names the same file(s) this would back up — not a backup:");
            for path in &paths {
                eprintln!("      {}", path.display());
            }
            eprintln!("  choose a directory this authority is not already inside.");
            std::process::ExitCode::FAILURE
        }
        // `ca backup` always calls `copy_all`, which never produces this
        // variant — only `copy_named` (`cargo rivet fleet objects backup`'s
        // own selection) does. Named rather than silently unreachable, so a
        // future edit that switched this call cannot fall through here
        // unnoticed.
        Err(other @ backup::Error::Missing(_)) => {
            eprintln!("rivet-provision: {other}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// **`ca restore <directory>`** — the inverse of `rivet-reset`, not of a
/// plain copy.
///
/// A restore that only moved files back to `~/.rivet/ca` would leave the
/// browser in exactly the state `rivet-reset` left it in: the files present
/// and trusted by nobody, and the one browser certificate that would work
/// invisible among whatever the store still holds. So this copies the
/// material back and then re-installs `root.crt` into the Root store and
/// `browser.pfx` into Personal — unless `--no-install` says to rehearse
/// instead. `SPEC-05` §4.
fn ca_restore(args: &[String]) -> std::process::ExitCode {
    let mut replace = false;
    let mut into_store = true;
    let mut positional: Vec<String> = Vec::new();
    for one in args {
        match one.as_str() {
            REPLACE => replace = true,
            NO_INSTALL => into_store = false,
            other if other.starts_with("--") => {
                eprintln!("rivet-provision: {other} is not a flag this verb takes");
                eprintln!("  the flags are {REPLACE} and {NO_INSTALL}");
                return std::process::ExitCode::FAILURE;
            }
            _ => positional.push(one.clone()),
        }
    }
    let [source] = positional.as_slice() else {
        eprintln!("usage: rivet-provision ca restore <directory> [{REPLACE}] [{NO_INSTALL}]");
        eprintln!("  <directory> holds a backup made by `rivet-provision ca backup`.");
        eprintln!("  {REPLACE} restores over an authority already at ~/.rivet/ca; without it,");
        eprintln!("  an existing authority is left alone and this refuses — restoring on top");
        eprintln!("  of a live authority is how a browser ends up offering two roots of the");
        eprintln!("  same name.");
        eprintln!("  {NO_INSTALL} writes the files and does NOT touch the certificate store —");
        eprintln!("  use it for every rehearsal of this ceremony.");
        eprintln!("  e.g. rivet-provision ca restore D:\\Backups\\rivet-ca");
        return std::process::ExitCode::FAILURE;
    };

    let Some(dest) = authority_directory() else {
        eprintln!("rivet-provision: no home directory to restore into");
        return std::process::ExitCode::FAILURE;
    };

    let copied = match backup::copy_all(std::path::Path::new(source), &dest, replace) {
        Ok(copied) => copied,
        Err(backup::Error::Empty(dir)) => {
            eprintln!("rivet-provision: {} holds nothing to restore", dir.display());
            return std::process::ExitCode::FAILURE;
        }
        Err(backup::Error::WouldOverwrite(existing)) => {
            eprintln!(
                "rivet-provision: {} already holds an authority. NOT restored.",
                dest.display()
            );
            eprintln!(
                "  restoring over a live authority is how a browser ends up offering two"
            );
            eprintln!("  roots of the same name — that is what made the certificate picker");
            eprintln!("  unusable on 2026-09-07.");
            print_existing(&existing);
            eprintln!(
                "  reset it first (rivet-reset), or pass {REPLACE} to restore over it anyway."
            );
            return std::process::ExitCode::FAILURE;
        }
        Err(backup::Error::Io(why)) => {
            eprintln!("rivet-provision: {why}");
            return std::process::ExitCode::FAILURE;
        }
        // See the identical arm in `ca_backup`: a real outcome, not a
        // defensive stub — `<directory>` naming `~/.rivet/ca` itself would
        // otherwise reach a same-file copy unguarded.
        Err(backup::Error::SameFile(paths)) => {
            eprintln!(
                "rivet-provision: {} names the same file(s) this would restore from — nothing to do:",
                dest.display()
            );
            for path in &paths {
                eprintln!("      {}", path.display());
            }
            return std::process::ExitCode::FAILURE;
        }
        // See the identical arm in `ca_backup`: `ca restore` also always
        // calls `copy_all`, which never produces this variant.
        Err(other @ backup::Error::Missing(_)) => {
            eprintln!("rivet-provision: {other}");
            return std::process::ExitCode::FAILURE;
        }
    };
    println!("  restored {source} to {}", dest.display());
    print_copied(&copied);

    if !into_store {
        println!();
        println!("  --no-install: the certificate store was NOT touched.");
        println!("  Double-click root.crt yourself: Current User, then Trusted Root");
        println!("  Certification Authorities. File browser.pfx: Current User, then Personal.");
        return std::process::ExitCode::SUCCESS;
    }

    if !dest.join("root.crt").exists() {
        println!();
        println!("  no root.crt was restored — nothing to install into the store");
        return std::process::ExitCode::SUCCESS;
    }

    reinstall(&dest)
}

/// Print what [`backup::copy_all`] copied.
fn print_copied(copied: &[backup::Copied]) {
    for c in copied {
        println!("      {:<24} {:>10} bytes", c.name, c.bytes);
    }
}

/// Print what a refused copy found already at the destination.
fn print_existing(existing: &[backup::Existing]) {
    for e in existing {
        eprintln!(
            "      {:<24} {:>10} bytes  {}",
            e.name,
            e.bytes,
            backup::age_text(e.age)
        );
    }
}

/// **Re-install the authority's root and the operator's browser identity —
/// the half that makes `restore` the inverse of `rivet-reset` rather than
/// the inverse of a plain copy.**
///
/// Called only from `ca_restore`, and only when `--no-install` was not
/// given. `SPEC-05` §4.
fn reinstall(dir: &std::path::Path) -> std::process::ExitCode {
    println!();
    println!("  Re-installing into the certificate store.");

    match browser::install_root(&dir.join("root.crt")) {
        browser::Placement::Placed(store) => println!("  root.crt filed into {store}"),
        browser::Placement::Unbuilt(how) => println!("  root.crt NOT installed: {how}"),
        browser::Placement::Refused(why) => println!("  root.crt NOT installed: {why}"),
    }

    let pfx = dir.join("browser.pfx");
    if !pfx.exists() {
        println!("  no browser.pfx in this backup — the operator identity was not restored");
        println!("  into the store. Mint one with rivet-provision ca, or restore a backup");
        println!("  that carries it.");
        return std::process::ExitCode::SUCCESS;
    }

    println!();
    println!("  browser.pfx needs its password to file it and to verify the key came in.");
    let password = ask_secret("password");
    match browser::place(&pfx, &password) {
        browser::Placement::Placed(store) => {
            println!("  filed into {store}");
            // **A return code is not evidence** — `SPEC-05` §4:
            // `certutil -importpfx` reports success while leaving a key
            // unattached when a copy of the certificate is already in the
            // store. This asks the store directly rather than trusting the
            // exit status above.
            match browser::private_key_attached(&pfx, &password) {
                Ok(true) => {
                    println!("  the private key is attached. Chrome will offer this identity.");
                }
                Ok(false) => {
                    println!();
                    println!("  THE CERTIFICATE IS IN THE STORE WITH NO PRIVATE KEY ATTACHED.");
                    println!("  A browser will silently decline to offer it — SPEC-05 §4.");
                    println!("  Remove it (rivet-reset does this) and restore again.");
                    return std::process::ExitCode::FAILURE;
                }
                Err(why) => {
                    println!();
                    println!("  FILED, BUT THE PRIVATE KEY COULD NOT BE VERIFIED: {why}");
                    return std::process::ExitCode::FAILURE;
                }
            }
        }
        browser::Placement::Unbuilt(how) => println!("  browser.pfx NOT installed: {how}"),
        browser::Placement::Refused(why) => {
            println!("  browser.pfx NOT installed: {why}");
            return std::process::ExitCode::FAILURE;
        }
    }
    std::process::ExitCode::SUCCESS
}

/// Write the answers, named for the hostname they configure.
///
/// # Errors
///
/// A sentence for the screen.
fn write_answers(fields: &[Field]) -> Result<std::path::PathBuf, String> {
    let name = fields
        .iter()
        .find(|f| f.label == "Hostname")
        .map_or("unnamed", |f| f.value.as_str());
    let path = save::path_for(name).ok_or_else(|| "no home directory to save in".to_owned())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(&path, save::render(fields)).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// **Write the composed document**, and never over something already there
/// without saying so.
///
/// # Errors
///
/// A sentence for the screen. `Ok(None)` is the operator declining to overwrite,
/// which is an answer and not a fault.
fn write_document(fields: &[Field], doc: &str) -> Result<Option<std::path::PathBuf>, String> {
    let name = value_of(fields, "Hostname");
    let name = if name.is_empty() { "unnamed".to_owned() } else { name };
    let path = save::document_path_for(&name)
        .ok_or_else(|| "no home directory to write in".to_owned())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }

    // **Named, not just counted.** A person about to replace a provisioning
    // document should see when it was written and how big it is, because the
    // question they are actually answering is *is this the one I made an hour
    // ago, or the one this machine is running on*.
    if path.exists() {
        println!();
        println!("  {} ALREADY EXISTS", path.display());
        if let Ok(meta) = std::fs::metadata(&path) {
            println!("  {} bytes", meta.len());
        }
        print!("  overwrite it? [y/N] ");
        let _ = std::io::stdout().flush();
        let mut yes = String::new();
        let read = std::io::stdin().read_line(&mut yes);
        // A closed stdin is not consent. `Ok(0)` is EOF.
        if !matches!(read, Ok(n) if n > 0) || !yes.trim().eq_ignore_ascii_case("y") {
            println!("  kept what was there; nothing was written");
            return Ok(None);
        }
    }

    std::fs::write(&path, doc).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Some(path))
}

/// **The fleet server's operator key for a MEMBER being staged**, read from
/// the Armada Registry at `~/.rivet/armada` (`ADR-059` names exactly one per
/// operator). This is the per-server fleet-operator public key `stage::compose`
/// adds to the member's `peers` so the member trusts the server that will
/// manage it — the fix for the "enrol never worked" trust gap (`docs/PLAN.md`
/// §183.2).
///
/// **`None` for anything that is not a member, and for a member whose Fleet's
/// key is not determinable here**: no Armada Registry yet, no Fleet in it,
/// several Fleets with no way yet to pick one (the interactive selector is a
/// later phase — the same 0/1/many `cargo rivet provision`'s own
/// `enroll_member_into_fleet` takes), or the one Fleet carrying no key (a server
/// provisioned before the key was recorded). Every one of those leaves the
/// member's peers staged exactly as before this existed. Best-effort and silent
/// by design: a missing key is a follow-up an operator fixes, never a failed
/// provision, so a registry that will not read is treated as no key rather than
/// an error surfaced here.
fn fleet_operator_key_for(fleet: &str) -> Option<String> {
    if fleet != "member" {
        return None;
    }
    fleet_operator_key_in(&rivet_root()?)
}

/// [`fleet_operator_key_for`]'s lookup, with the credential root handed in —
/// `<root>/armada` — so `rivet-provision stage --at DIR` reads the same Armada
/// the interview reads under `RIVET_AT`.
fn fleet_operator_key_in(root: &std::path::Path) -> Option<String> {
    let armada = rivetos_register::armada::store::load(&root.join("armada")).ok()?;
    // 0 / 1 / many, without unwrap — a single Fleet is the one this member
    // joins; several is the selector's job, which is not built yet.
    let mut fleets = armada.fleets.values();
    match (fleets.next(), fleets.next()) {
        (Some(only), None) => only.server.operator_key.clone(),
        _ => None,
    }
}

/// Publish `text` to `path`, replacing whatever bundle is there without ever
/// leaving `path` holding a truncated or mixed one.
///
/// Shared by every writer of an `ADR-049` staging bundle in this binary:
/// `write_staging` for the interview's own bundle and
/// `stage_document::write_bundle` for `rivet-provision stage`'s.
///
/// Writes to a fixed, private temporary file beside `path`, flushes it, then
/// renames it over `path`; every earlier step writes to the temporary alone,
/// so the rename is the single step that ever opens `path` itself. A failure
/// before the rename removes only the temporary and leaves `path` exactly as
/// it was. The temporary's name is fixed rather than unique per call, so one
/// left behind by an interrupted previous run is simply overwritten by the
/// next rather than accumulating.
///
/// `path` is typically on the FAT partition the board's boot firmware reads
/// (`ADR-002` §4), so this uses only `std::fs`, which is what a FAT driver
/// supports on Windows and on Linux alike. FAT itself carries no journal: a
/// `rename` there is one directory-entry update rather than the multi-block
/// truncating write this replaces, which is why it closes the defect — but
/// neither this function nor FAT promises the renamed entry has reached the
/// physical medium before a power cut. `std::fs` has no directory-flush
/// primitive for a FAT volume the way `rivet_store::atomic::commit` has for
/// an `ext4` `/state`.
///
/// # Errors
///
/// Whatever `std::fs` reports, from creating the temporary, writing it,
/// flushing it, or renaming it over `path`.
fn publish_bundle(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    let mut temp_name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    temp_name.push(".new");
    let temp = path.with_file_name(temp_name);

    let written = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(e);
    }
    Ok(())
}

/// **Write the staging bundle** — the material the card carries for this
/// machine's applications, `ADR-049`.
///
/// It runs after the document has been written and checked, because the two are
/// one card and the document is the thing a person can still change their mind
/// about. Answers whether the menu may exit successfully.
///
/// **A failure here is fatal to the run and says why.** The alternative is a
/// card built from a document that was written and material that was not, which
/// is the fleet server that boots with an empty register — the exact state
/// `SPEC-05` §1 exists to end, reached by a tool that reported success.
fn write_staging(fields: &[Field], document: &std::path::Path) -> bool {
    let hostname = value_of(fields, "Hostname");
    let identity_answer = value_of(fields, "Identity");
    let identity = (!identity_answer.trim().is_empty()).then(|| std::path::PathBuf::from(identity_answer.trim()));
    let Some(authority) = authority_directory() else {
        println!("  NOT staged: no home directory, so there is no authority to read");
        return false;
    };
    let name = if hostname.is_empty() { "unnamed".to_owned() } else { hostname.clone() };
    // **What a previous `w` staged for this machine**, so its operator
    // credential is reused and the peers file gains no second key.
    let staged_before = save::staging_path_for(&name).and_then(|p| std::fs::read_to_string(p).ok());

    // **The fleet server's operator key for a member**, off the Armada Registry
    // — `stage::compose` adds it to the member's peers so the server can manage
    // it (`docs/PLAN.md` §183.2). Best-effort and `None` for anything else.
    let fleet = value_of(fields, "Fleet");
    let operator_key = fleet_operator_key_for(&fleet);

    // **Fleet-root mode**, when the first-provision guard recorded it: a
    // server's card carries the request instead of a leaf from this
    // workstation (`ADR-063` §4).
    let fleet_root = if fleet == "server" {
        match fleet_root_request(&authority, &hostname) {
            Ok(request) => request,
            Err(why) => {
                println!("  NOTHING STAGED: {why}");
                return false;
            }
        }
    } else {
        None
    };

    let machine = stage::Machine {
        fleet: &fleet,
        hostname: &hostname,
        listen: listen_for(&fleet),
        method: &value_of(fields, "Addressing"),
        address: &value_of(fields, "Address"),
        gateway: &value_of(fields, "Gateway"),
        authority: &authority,
        identity: identity.as_deref(),
        staged_before: staged_before.as_deref(),
        operator_key: operator_key.as_deref(),
        fleet_root: fleet_root.as_ref(),
    };

    let plan = match stage::compose(&machine) {
        Ok(plan) => plan,
        Err(why) => {
            println!("  NOTHING STAGED: {why}");
            return false;
        }
    };
    for note in &plan.notes {
        println!("{note}");
    }
    if plan.items.is_empty() {
        return true;
    }

    let Some(path) = save::staging_path_for(&name) else {
        println!("  NOT staged: no home directory to write in");
        return false;
    };
    if let Some(dir) = path.parent()
        && let Err(why) = std::fs::create_dir_all(dir)
    {
        println!("  NOT staged: {}: {why}", dir.display());
        return false;
    }
    let text = rivet_core::staging::render(&plan.items);
    if let Err(why) = publish_bundle(&path, &text) {
        println!("  NOT staged: {}: {why}", path.display());
        return false;
    }

    // **Read back and parsed, for the reason the document is** — `SPEC-05` §4.
    // `render` produced the string in memory; this asks whether the FILE is a
    // bundle, which is the question `cmd/rivet-image` will ask next.
    match std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|back| rivet_core::staging::parse(&back).map_err(|e| e.to_string()))
    {
        Ok(back) => {
            println!("  staged  {} ({} file(s))", path.display(), back.len());

            // **Fix 2, and deliberately AFTER the card material is confirmed
            // written and readable.** `stage::compose` only composes —
            // nothing is written until everything is — so the write of this
            // machine's own leaf into the WORKSTATION's CA directory happens
            // here, once the run this far has definitely succeeded. A
            // failure here does not undo the card staging above (`false`
            // there is reserved for the card material itself, `SPEC-05` §1's
            // "empty register" failure); it is reported plainly instead, and
            // Fix 3 (`cargo rivet fleet`'s own self-heal for a board
            // provisioned before this existed) is the reason it is safe to
            // treat as non-fatal rather than unwind everything above it.
            if let Some((subject, leaf)) = &plan.workstation_leaf {
                match write_server_leaf(&authority, subject, leaf) {
                    Ok(written) => {
                        for path in &written {
                            println!("  {}", path.display());
                        }
                        println!(
                            "  {subject}'s own leaf is also this workstation's copy now — \
                             `cargo rivet fleet`'s stage material (fleet-tls) step reads it \
                             for a later deploy to {subject}"
                        );
                    }
                    Err(why) => {
                        println!("  WORKSTATION COPY NOT WRITTEN: {why}");
                        println!(
                            "  the card still carries {subject}'s leaf and this machine will \
                             boot fine; a later `cargo rivet fleet` deploy to it will offer to \
                             mint this file itself"
                        );
                    }
                }
            }

            if let Some(next) = after_staging(orchestrated(), document, &identity_answer) {
                println!("{next}");
            }
            true
        }
        Err(why) => {
            println!("  STAGED FILE WILL NOT READ BACK: {why}");
            false
        }
    }
}

/// **Read the written document back off the disk and let the board's own parser
/// rule on it** — `SPEC-05` §4.
///
/// `compose` parses the string it built, which proves the composition. This
/// proves the FILE, which is the different and load-bearing claim: what goes on
/// a card is bytes on a disk, and *"it looked right"* is not a check.
///
/// Returns whether it parsed.
fn check_written(path: &std::path::Path) -> bool {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(why) => {
            println!("  CANNOT READ BACK {}: {why}", path.display());
            return false;
        }
    };
    match rivet_config::Document::parse(&text) {
        Ok(parsed) => {
            let listen = parsed.effective_control_plane().listen().to_owned();
            println!(
                "  parsed by rivet-config — the same parser the board refuses with (ADR-004)"
            );
            println!(
                "    hostname       {}",
                parsed
                    .hostname()
                    .map_or_else(|| "(derived by the machine)".to_owned(), ToString::to_string)
            );
            println!("    interfaces     {}", parsed.network().len());
            println!("    services       {}", parsed.services().len());
            println!("    control plane  {listen}");
            true
        }
        // **The schema's own words**, as everywhere else in this tool.
        Err(why) => {
            println!("  THE WRITTEN FILE DOES NOT PARSE: {why:?}");
            println!("  Do not put it on a card. This is the check existing to catch exactly");
            println!("  this, and it caught something.");
            false
        }
    }
}

/// Ask whether to carry on from something saved earlier.
///
/// **Only asked when there is something to ask about.** A prompt that always
/// appears and usually has one answer is a keystroke, not a question.
fn offer_to_resume() -> Option<std::path::PathBuf> {
    let saved = save::saved();
    if saved.is_empty() {
        return None;
    }
    println!();
    for (i, p) in saved.iter().take(9).enumerate() {
        println!("  {}  {}", i + 1, p.file_stem()?.to_string_lossy());
    }
    print!("  resume which? [enter for none] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer).ok()?;
    let index = answer.trim().parse::<usize>().ok()?.checked_sub(1)?;
    saved.get(index).cloned()
}

/// Mint a fleet server's certificate, from a root already on disk.
///
/// **Separate from `ca` because the hostname is not known when the authority
/// is.** A browser validates the name it asked for, so this certificate can
/// only be made once that name exists.
fn mint_server(args: &[String]) -> std::process::ExitCode {
    // `--names <list>` carries the literal list, so the positional shape says
    // which mode this is: two positionals is a host to compose from, one is a
    // directory and the names came off the flag.
    let mut literal: Option<String> = None;
    let mut positional: Vec<String> = Vec::new();
    let mut rest = args.iter();
    while let Some(one) = rest.next() {
        match one.as_str() {
            NAMES => {
                let Some(list) = rest.next() else {
                    eprintln!("rivet-provision: {NAMES} takes a comma-separated list of names");
                    return std::process::ExitCode::FAILURE;
                };
                literal = Some(list.clone());
            }
            other if other.starts_with("--") => {
                eprintln!("rivet-provision: {other} is not a flag this verb takes");
                eprintln!("  the only one is {NAMES}");
                return std::process::ExitCode::FAILURE;
            }
            _ => positional.push(one.clone()),
        }
    }

    let (host, dir) = match (literal.as_ref(), positional.as_slice()) {
        (None, [host, dir]) => (Some(host.clone()), dir.clone()),
        (Some(_), [dir]) => (None, dir.clone()),
        _ => {
            eprintln!("usage: rivet-provision ca server <hostname> <directory>");
            eprintln!(
                "       rivet-provision ca server {NAMES} <name,name,...> <directory>"
            );
            eprintln!("  e.g. rivet-provision ca server arbor ~/.rivet/ca");
            eprintln!();
            eprintln!("  THE FIRST FORM IS THE ONE TO USE. <hostname> is the machine's name");
            eprintln!("  and nothing else; this composes <hostname>.<suffix> for EVERY suffix");
            eprintln!("  the authority permits, read out of root.crt. A machine reached over");
            eprintln!("  mDNS and over DNS answers to two names and needs both in ONE");
            eprintln!("  certificate, or one way in works and the other shows a browser");
            eprintln!("  warning with nothing on screen saying why.");
            eprintln!();
            eprintln!("  {NAMES} takes a literal list instead, for somebody who means exactly");
            eprintln!("  those names. It is opt-in because typing one name used to be the");
            eprintln!("  DEFAULT, and it silently replaced a three-name leaf with a one-name");
            eprintln!("  one on a production fleet server (findings §3.3).");
            eprintln!();
            eprintln!("  Every name must fall under a suffix the authority permits; the");
            eprintln!("  refusal names the ones it does.");
            return std::process::ExitCode::FAILURE;
        }
    };
    let dir = &dir;

    let root = std::path::Path::new(dir);
    let (certificate, key) = match (
        std::fs::read_to_string(root.join("root.crt")),
        std::fs::read_to_string(root.join("root.key")),
    ) {
        (Ok(c), Ok(k)) => (c, k),
        _ => {
            eprintln!("rivet-provision: no root.crt and root.key in {dir}");
            eprintln!("  mint one first: rivet-provision ca <suffix> <label> {dir}");
            return std::process::ExitCode::FAILURE;
        }
    };

    // **Composed from the authority, exactly as the menu composes it.**
    //
    // `ca server "rivet-01.local"` used to mint a certificate with a single
    // SAN and overwrite the correct three-name one, because the verb took the
    // list literally while `mint_for_host` composed it — two adapters, one
    // request, different certificates (`ADR-006` §3). Only `rivet-01.local`
    // then validated on the production fleet server.
    //
    // The suffixes are read out of `root.crt` and not out of the `suffix` note
    // beside it, for the reason `mint_for_host` gives: a certificate cannot
    // drift from itself, and the note is read by nothing else.
    let names = match host {
        Some(host) => {
            let suffixes = match ca::permitted_suffixes(&certificate) {
                Ok(s) if !s.is_empty() => s,
                Ok(_) => {
                    eprintln!(
                        "rivet-provision: this authority constrains no DNS names, so it cannot \
                         vouch for a host"
                    );
                    return std::process::ExitCode::FAILURE;
                }
                Err(why) => {
                    eprintln!("rivet-provision: {why}");
                    return std::process::ExitCode::FAILURE;
                }
            };
            // **A suffix already typed is not typed twice.** `rivet-01.local`
            // is what a person reaches for, and composing on top of it would
            // ask the authority for `rivet-01.local.local`. Stripping it means
            // the command that produced the one-name leaf now produces the
            // right certificate rather than a refusal.
            let bare = strip_permitted_suffix(&host, &suffixes);
            if bare != host {
                println!("  {host} names a suffix; composing from {bare}");
            }
            // **A HOSTNAME IS ONE LABEL.** What is left after a permitted
            // suffix comes off must be, or the operator typed a name under a
            // suffix this authority does not permit — and composing on top of
            // it would turn `demo.example.com` into `demo.example.com.local`,
            // which the constraint permits and nobody asked for. The refusal
            // names what would have worked, because the list was frozen at a
            // mint performed months ago in a file nobody opens.
            if bare.contains('.') {
                eprintln!(
                    "rivet-provision: {host:?} is not under any suffix this authority permits: {}",
                    suffixes.join(", ")
                );
                eprintln!(
                    "  <hostname> is the machine's name alone - this composes the rest. Pass \
                     {NAMES} to name a certificate's SANs literally."
                );
                return std::process::ExitCode::FAILURE;
            }
            every_name_for(&bare, &suffixes)
        }
        // **The same parser the authority's own suffix list uses.** One idiom
        // for one shape — a person who has typed a comma-separated list once
        // should not discover the second one is different.
        None => split_suffixes(literal.as_deref().unwrap_or_default()),
    };
    if names.is_empty() {
        eprintln!("rivet-provision: no hostname given");
        return std::process::ExitCode::FAILURE;
    }
    println!("  minting for {}", names.join(", "));

    // **The subject is the first name given**, matching `mint_for_host`, and it
    // is also what the files are named after. Nothing authenticates by the
    // subject (`ADR-028` §4.5); every name that matters is in the SAN list.
    let subject = &names[0];
    let minted = match ca::issue_from(&certificate, &key, &names, subject) {
        Ok(m) => m,
        Err(why) => {
            eprintln!("rivet-provision: {why}");
            return std::process::ExitCode::FAILURE;
        }
    };

    match write_server_leaf(root, subject, &minted) {
        Ok(written) => {
            for path in &written {
                println!("  {}", path.display());
            }
        }
        Err(why) => {
            eprintln!("rivet-provision: {why}");
            return std::process::ExitCode::FAILURE;
        }
    }
    println!();
    println!("  for the fleet server answering to {}", names.join(", "));
    std::process::ExitCode::SUCCESS
}

/// Write a machine's leaf into the CA directory as `server-<subject>.crt` and
/// `server-<subject>.key` — **the one place that names this file shape**,
/// shared by [`mint_server`] (`rivet-provision ca server`) and provisioning's
/// own `stage::compose`.
///
/// **Added for Fix 2 (`cargo rivet fleet`'s own gap, `.rulings/` unruled at
/// time of writing).** `cargo rivet fleet`'s "stage material (fleet-tls)"
/// step reads exactly `~/.rivet/ca/server-<name>.local.crt` off the
/// workstation before it will deploy a TLS-serving application to a fleet
/// member. Provisioning already mints this machine's leaf and stages it onto
/// the card (`stage::compose`); calling this with the SAME `leaf` — never a
/// second, independently-keyed mint — is what leaves the workstation copy a
/// later deploy needs, so a member provisioned from here on needs no separate
/// `ca server` step before its first TLS deploy.
///
/// # Errors
///
/// Whatever `std::fs::write` says, naming the path it failed at.
fn write_server_leaf(
    dir: &std::path::Path,
    subject: &str,
    leaf: &ca::Minted,
) -> Result<Vec<std::path::PathBuf>, String> {
    let mut written = Vec::new();
    for (ext, body) in [("crt", &leaf.certificate), ("key", &leaf.key)] {
        let path = dir.join(format!("server-{subject}.{ext}"));
        std::fs::write(&path, body).map_err(|why| format!("{}: {why}", path.display()))?;
        written.push(path);
    }
    Ok(written)
}

/// **The credential root every path in this tool hangs off** — see this
/// module's own `RIVET_AT` section for the defect this closes.
///
/// `RIVET_AT` first (set by `cargo rivet provision`'s `SubprocessInterview`
/// to the same root that orchestrator already resolved for itself, whether
/// that came from `--at DIR` or its own default), then this tool's own
/// long-standing default: `$HOME/.rivet`, or `$USERPROFILE\.rivet`. Running
/// this binary directly, with nobody setting `RIVET_AT`, is unaffected.
#[must_use]
fn rivet_root() -> Option<std::path::PathBuf> {
    resolve_rivet_root(std::env::var_os("RIVET_AT"), std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")))
}

/// [`rivet_root`]'s own decision, pure — no environment in the loop, so it
/// is provable with two values and nothing else.
///
/// **`var_os` answers `Some("")` for a variable set to nothing, and a shell
/// spells *unset* that way in practice** — `cmd/cargo-rivet/src/credentials.rs`'s
/// own `set_to_something` learned this the expensive way; both `at` and
/// `home` are filtered here on the identical rule.
fn resolve_rivet_root(
    at: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Option<std::path::PathBuf> {
    if let Some(at) = at.filter(|v| !v.is_empty()) {
        return Some(std::path::PathBuf::from(at));
    }
    let home = home.filter(|v| !v.is_empty())?;
    Some(std::path::PathBuf::from(home).join(".rivet"))
}

/// Where a fleet's authority lives.
fn authority_directory() -> Option<std::path::PathBuf> {
    Some(rivet_root()?.join("ca"))
}

/// One field's answer.
fn value_of(fields: &[Field], label: &str) -> String {
    fields
        .iter()
        .find(|f| f.label == label)
        .map_or_else(String::new, |f| f.value.clone())
}

/// Ask a question with a default, terse.
/// **Ask for a secret without echoing it.**
///
/// The maintainer's permission is explicit — a password prompt from
/// `rivet-provision` at any point is acceptable to him (.rulings/ R-280) — and a
/// prompt that then printed it across the screen would be taking that
/// permission and spending it badly.
///
/// # Why this is twenty lines of FFI and not a crate
///
/// `ADR-009` §3's ladder prefers an existing crate, and its §5 gate is what
/// overrides it here: this binary mints and holds `root.key`, the fleet's whole
/// trust, so every dependency it gains is one more thing with the opportunity
/// to read one. Three console calls are the cheaper side of that trade — the
/// same argument `cmd/cargo-rivet`'s ELF reader makes.
///
/// **A console this cannot silence is a console it does not lie about.** If the
/// mode cannot be changed, the prompt says so rather than reading a secret
/// while the person believes it is hidden.
/// **Put the operator's identity where their browser will find it.**
///
/// The maintainer drew the line himself, and it is the right line: a one-time
/// double-click on the root is an acceptable setup task, but the browser's
/// identity has to go into one specific store, which a user has no natural
/// reason to know about (.rulings/ R-280).
///
/// So the authority stays a deliberate human act and the identity — which the
/// person already owns — is filed for them. Nothing here changes what this
/// computer trusts.
///
/// Failure is reported and never fatal: the bundle is on disk either way, and a
/// person who has it can finish by hand.
///
/// # `--no-install` is the one way out, and it exists so rehearsal is possible
///
/// `certutil` lives in the system directory, so a run cannot be made harmless
/// by taking it off `PATH`. Without a flag there was **no dry run of the
/// ceremony at all**, and every practice run left a certificate in the real
/// Personal store (`SPEC-05` §2). With it, `browser.pfx` is still written and
/// the store is not opened.
fn enroll(dir: &std::path::Path, identity: &ca::Minted, into_store: bool) {
    println!();
    if into_store {
        println!("  Your browser identity needs to be in the Personal store for THIS user.");
        println!("  That is the one choice a wizard cannot make for you, so this will do it.");
    } else {
        println!("  --no-install: browser.pfx will be written and the certificate store");
        println!("  will NOT be opened. No browser can see this identity until you file it.");
    }
    println!();
    println!("  A password protects browser.pfx on disk. Press Enter for none - the file");
    println!("  is your private key either way, so somewhere only you can read.");
    let password = ask_secret("password");

    if !into_store {
        match browser::write_bundle(dir, identity, &password) {
            Ok(path) => {
                println!();
                println!("  {} is written and NOT installed.", path.display());
                println!("  --no-install was given, so nothing was added to any store.");
                println!("  File it when you mean to: double-click, Current User, then Personal.");
            }
            Err(why) => {
                println!();
                println!("  no bundle was written: {why}");
            }
        }
        return;
    }

    match browser::install(dir, identity, &password) {
        Ok(browser::Placement::Placed(store)) => {
            println!();
            println!("  filed into {store}");
            println!("  Chrome will offer it the next time it asks. Restart it if it does not.");
        }
        Ok(browser::Placement::Unbuilt(how)) => {
            println!();
            println!("  browser.pfx is written and NOT installed.");
            println!("  {how}");
        }
        Ok(browser::Placement::Refused(why)) => {
            println!();
            println!("  browser.pfx is written and was NOT installed: {why}");
            println!("  Double-click it: Current User, then Personal.");
        }
        Err(why) => {
            println!();
            println!("  no bundle was written: {why}");
        }
    }
}

fn ask_secret(label: &str) -> String {
    #[cfg(windows)]
    let quiet = windows_console::hush();
    #[cfg(not(windows))]
    let quiet: Option<()> = None;

    if quiet.is_none() {
        println!("  (this terminal will show what you type)");
    }
    print!("  {label}: ");
    let _ = std::io::stdout().flush();
    let mut given = String::new();
    let read = std::io::stdin().read_line(&mut given);

    #[cfg(windows)]
    windows_console::restore(quiet);

    println!();
    if read.is_err() {
        return String::new();
    }
    given.trim_end().to_owned()
}

/// Turning console echo off, and putting it back.
///
/// `SetConsoleMode` with `ENABLE_ECHO_INPUT` cleared is how Windows hides a
/// password, and it must be restored whatever happens — a terminal left with
/// echo off outlives this process and looks, to the person using it, like their
/// keyboard has stopped working.
#[cfg(windows)]
mod windows_console {
    /// `STD_INPUT_HANDLE`.
    const STD_INPUT: u32 = -10_i32 as u32;
    /// `ENABLE_ECHO_INPUT`.
    const ECHO: u32 = 0x0004;

    unsafe extern "system" {
        fn GetStdHandle(which: u32) -> isize;
        fn GetConsoleMode(handle: isize, mode: *mut u32) -> i32;
        fn SetConsoleMode(handle: isize, mode: u32) -> i32;
    }

    /// Clear echo, returning the mode to put back — or `None` if this is not a
    /// console (a pipe, a redirect), where there is nothing to silence.
    pub fn hush() -> Option<u32> {
        // SAFETY: `GetStdHandle` returns a handle this process already owns and
        // `GetConsoleMode` writes one `u32` through a pointer to a live local.
        unsafe {
            let handle = GetStdHandle(STD_INPUT);
            let mut mode = 0u32;
            if GetConsoleMode(handle, &raw mut mode) == 0 {
                return None;
            }
            if SetConsoleMode(handle, mode & !ECHO) == 0 {
                return None;
            }
            Some(mode)
        }
    }

    /// Put the mode back. A no-op if `hush` found no console.
    pub fn restore(mode: Option<u32>) {
        let Some(mode) = mode else { return };
        // SAFETY: as above, with a mode this module read from the same handle.
        unsafe {
            let handle = GetStdHandle(STD_INPUT);
            SetConsoleMode(handle, mode);
        }
    }
}

/// Read the suffix list, and **do not mint one the operator has not seen**.
///
/// # The failure this exists to prevent, which happened to a person
///
/// The prompt offered `local,my-fleet.home.arpa` and the operator typed
/// `my-fleet.lan`. That **replaced** the list rather than adding to it, which is
/// what a typed value should do — and the authority it would have minted could
/// vouch for `*.my-fleet.lan` and nothing else, with no `.local` at all. `.local`
/// is what actually resolves today, since `rivet-mdnsd` announces it and there
/// is no DNS server yet.
///
/// Nothing on screen said so. The only clue was a note that `.lan` needs a DNS
/// server, which reads as advice rather than as *"`.local` is not in your
/// list."* On the one field a mint cannot take back, that gap is discovered
/// months later in a browser, and the repair is a new root re-trusted on every
/// device (`ADR-011` §3).
///
/// # So two changes, and the second is the load-bearing one
///
/// **`+name` appends** to what is offered, because *add mine to yours* was
/// plainly the intent and the syntax for it did not exist.
///
/// **The composed list is rendered and confirmed before anything is minted.**
/// That is what actually closes the hole: it catches a replacement the operator
/// meant as an addition, a typo, a dropped entry — every way the list can be
/// wrong, including ways nobody has thought of, because the operator reads the
/// thing itself rather than reasoning about what their input did to it.
fn ask_for_suffixes(fleet: &str) -> Vec<String> {
    let offered = default_suffixes(fleet);
    loop {
        println!("  type a list to REPLACE what is offered, or +name to ADD to it");
        let given = ask("suffixes", &offered.join(","));

        let chosen = if let Some(extra) = given.strip_prefix('+') {
            let mut out = offered.clone();
            for one in split_suffixes(extra) {
                if !out.contains(&one) {
                    out.push(one);
                }
            }
            out
        } else {
            split_suffixes(&given)
        };

        // An empty list is an authority that constrains nothing, which
        // `ca::Authority::root` refuses — better to say so here than to let the
        // refusal arrive after three more questions.
        if chosen.is_empty() {
            println!("  at least one suffix is needed");
            continue;
        }

        println!();
        println!("  this authority will vouch for these, and NOTHING else:");
        for one in &chosen {
            println!("      {one}");
        }
        if !ask("keep these? [Y/n]", "y").eq_ignore_ascii_case("n") {
            return chosen;
        }
        println!();
    }
}

fn ask(label: &str, default: &str) -> String {
    print!("  {label} [{default}] ");
    let _ = std::io::stdout().flush();
    let mut given = String::new();
    if std::io::stdin().read_line(&mut given).is_err() {
        return default.to_owned();
    }
    let given = given.trim();
    if given.is_empty() {
        default.to_owned()
    } else {
        given.to_owned()
    }
}

/// What the first-provision guard concluded (`ADR-063` §8).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Guarded {
    /// This workstation already has an authority, and nothing was asked.
    Present,
    /// The operator said this is a new fleet, and its authority was minted.
    Minted,
    /// The operator said this is a new fleet whose root is minted on its
    /// fleet server (`ADR-063` §4). Nothing was minted here; the fleet's
    /// name and suffixes were recorded for the server's card.
    FleetRoot,
    /// The operator is joining an existing fleet. Nothing was minted, and the
    /// run stops after saying what to carry across instead.
    Joining,
    /// The operator left the question with `x`. Nothing was minted.
    Exited,
    /// There was no answer to act on — no terminal and no `--new-fleet`, or
    /// input closed mid-question. Nothing was minted; the sentence says why.
    Refused(String),
}

/// The first-provision guard's one question, answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FirstFleet {
    New,
    /// A new fleet in fleet-root mode: its root is minted on its server.
    FleetRoot,
    Join,
    Exit,
}

/// **Where a fleet-root mode choice is recorded**, inside the authority
/// directory: the fleet's name and the suffixes its server's root will be
/// constrained to, one `key = value` per line — `rivet_core::fleet_root`'s
/// request without its `version` and `host`, which the server's own document
/// supplies at `w`. Its presence answers the first-provision guard as an
/// authority's does: this workstation's fleet has a root, and it lives on the
/// server.
const FLEET_ROOT_CHOICE: &str = "fleet-root.txt";

/// The fleet-root request for `hostname`, when this workstation chose
/// fleet-root mode at the first-provision guard.
///
/// # Errors
///
/// A recorded choice that does not compose a legal request.
fn fleet_root_request(
    authority: &std::path::Path,
    hostname: &str,
) -> Result<Option<rivet_core::fleet_root::Request>, String> {
    let path = authority.join(FLEET_ROOT_CHOICE);
    let Ok(recorded) = std::fs::read_to_string(&path) else {
        return Ok(None);
    };
    let text = format!(
        "version = {}\nhost = {hostname}\n{recorded}",
        rivet_core::fleet_root::VERSION
    );
    rivet_core::fleet_root::parse(&text)
        .map(Some)
        .map_err(|why| format!("{}: {why}", path.display()))
}

/// Ask the fleet's name and its suffixes, and record them in `dir` for the
/// server's card — the fleet-root half of the first-provision guard. Mints
/// nothing.
///
/// # Errors
///
/// A name that is not one DNS label, or suffixes outside the fleet's names.
fn ask_and_record_fleet_root(dir: &std::path::Path) -> Result<(), String> {
    let label = dns_label(&ask("fleet name", DEFAULT_FLEET))
        .ok_or_else(|| "a fleet name must make one DNS label".to_owned())?;
    let offered = rivet_core::fleet_root::default_suffixes(&label).join(", ");
    let suffixes = split_suffixes(&ask("DNS suffixes", &offered));
    record_fleet_root(dir, &label, &suffixes)?;
    println!(
        "  recorded fleet {label:?} in fleet-root mode: its root is minted on its fleet server's \
         first boot, and the server's card carries the request ({})",
        dir.join(FLEET_ROOT_CHOICE).display()
    );
    Ok(())
}

/// Write the fleet-root choice, checked as the board will check it.
fn record_fleet_root(dir: &std::path::Path, fleet: &str, suffixes: &[String]) -> Result<(), String> {
    // Checked with a stand-in host: the fleet and the suffixes are what is
    // recorded, and the host comes from each server's document at `w`.
    let checked = rivet_core::fleet_root::Request::new(fleet, "host", suffixes)
        .map_err(|why| why.to_string())?;
    let mut text = format!("fleet = {}\n", checked.fleet);
    for suffix in &checked.suffixes {
        text.push_str(&format!("suffix = {suffix}\n"));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(FLEET_ROOT_CHOICE);
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// The refusal for a run with no terminal and no `--new-fleet`.
const NO_ANSWER_REFUSAL: &str = "NOTHING WAS MINTED. This workstation has no certificate \
     authority, and a run with no terminal cannot be asked whether it is starting a new fleet \
     or joining an existing one. Pass --new-fleet to start a new fleet here, or, to join one, \
     carry ~/.rivet and rivet-fleet's register across from a workstation already in it.";

/// **Which answer a line of input gives**, pure so every spelling is provable
/// with no terminal. A number or the choice's own word; there is no default,
/// because a bare Return is not a decision to mint a fleet's trust.
///
/// # Errors
///
/// Anything else, as a sentence for the screen.
fn first_fleet_from_answer(answer: &str) -> Result<FirstFleet, String> {
    match answer.trim().to_ascii_lowercase().as_str() {
        "1" | "new" | "new fleet" => Ok(FirstFleet::New),
        "2" | "join" => Ok(FirstFleet::Join),
        "3" | "fleet-root" | "fleet root" => Ok(FirstFleet::FleetRoot),
        "x" => Ok(FirstFleet::Exit),
        "" => Err("choose 1, 2 or 3 — nothing is minted until you do".to_owned()),
        other => Err(format!("{other:?} is not 1, 2, 3 or x")),
    }
}

/// Where `rivet-fleet`'s register lives on this platform (`ADR-062` §5).
fn register_location() -> &'static str {
    if cfg!(windows) {
        r"%APPDATA%\rivet-fleet\register"
    } else if cfg!(target_os = "macos") {
        "~/Library/Application Support/rivet-fleet/register"
    } else {
        "~/.config/rivet-fleet/register"
    }
}

/// **The first-provision guard** — `ADR-063` §8.
///
/// A workstation with no authority in `dir` is asked whether it is starting a
/// new fleet or joining an existing one, and `mint` runs only on the answer
/// *new*. A workstation that already has an authority is never asked. The
/// failure this closes is a second workstation's first run minting a second
/// authority beside the fleet's real one, silently, because nothing asked.
///
/// `interactive` is whether a person is at the terminal; `new_fleet` is
/// `--new-fleet`, the answer given in advance. Without a terminal and without
/// the flag this refuses rather than mints. `ask_choice` reads one line and
/// answers `None` when input has closed.
///
/// # Errors
///
/// Whatever `mint` returns, once the operator has chosen a new fleet.
fn first_provision_guard(
    dir: &std::path::Path,
    interactive: bool,
    new_fleet: bool,
    ask_choice: &mut dyn FnMut() -> Option<String>,
    mint: &mut dyn FnMut(&std::path::Path) -> Result<(), String>,
    record_fleet_root: &mut dyn FnMut(&std::path::Path) -> Result<(), String>,
) -> Result<Guarded, String> {
    if dir.join("root.crt").exists() || dir.join(FLEET_ROOT_CHOICE).exists() {
        return Ok(Guarded::Present);
    }
    if new_fleet {
        mint(dir)?;
        return Ok(Guarded::Minted);
    }
    if !interactive {
        return Ok(Guarded::Refused(NO_ANSWER_REFUSAL.to_owned()));
    }

    println!();
    println!("  This workstation has no certificate authority yet ({}).", dir.display());
    println!("    1)  start a new fleet — mint its authority here");
    println!("    2)  join an existing fleet — mint nothing");
    println!("    3)  start a new fleet whose root is minted on its fleet server (fleet-root mode)");
    println!("    x)  exit — leave without choosing");
    let choice = loop {
        print!("  new fleet, or join one? [1, 2, 3, or x]: ");
        let _ = std::io::stdout().flush();
        let Some(line) = ask_choice() else {
            return Ok(Guarded::Refused(
                "NOTHING WAS MINTED. Input closed before saying whether this is a new fleet or \
                 an existing one."
                    .to_owned(),
            ));
        };
        match first_fleet_from_answer(&line) {
            Ok(choice) => break choice,
            Err(why) => println!("  {why}"),
        }
    };

    match choice {
        FirstFleet::New => {
            mint(dir)?;
            Ok(Guarded::Minted)
        }
        FirstFleet::FleetRoot => {
            record_fleet_root(dir)?;
            Ok(Guarded::FleetRoot)
        }
        FirstFleet::Join => {
            println!();
            println!("  Nothing was minted. Joining a fleet from here is not built yet, so a");
            println!("  workstation joins today by carrying the existing one's state across:");
            println!("    1. copy ~/.rivet from a workstation already in the fleet — its");
            println!("       authority, identities, Armada, repository and documents;");
            println!("    2. copy rivet-fleet's register, which is not under ~/.rivet:");
            println!("       {}", register_location());
            println!("  Then run this again: with the authority in place it is not asked.");
            Ok(Guarded::Joining)
        }
        FirstFleet::Exit => {
            println!("  exited — nothing was minted.");
            Ok(Guarded::Exited)
        }
    }
}

/// **Mint the authority if there is not one**, once the operator has said this
/// is a new fleet — [`first_provision_guard`] with this terminal and
/// [`ask_and_mint_authority`].
///
/// # Errors
///
/// A sentence for the screen, from a mint the operator asked for.
fn ensure_authority(new_fleet: bool) -> Result<Guarded, String> {
    let dir = authority_directory().ok_or_else(|| "no home directory".to_owned())?;
    let interactive = std::io::IsTerminal::is_terminal(&std::io::stdin());
    let mut ask_choice = || {
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line),
        }
    };
    first_provision_guard(
        &dir,
        interactive,
        new_fleet,
        &mut ask_choice,
        &mut ask_and_mint_authority,
        &mut ask_and_record_fleet_root,
    )
}

/// Ask what an authority cannot be minted without, then mint it into `dir`.
///
/// # Errors
///
/// A sentence for the screen.
fn ask_and_mint_authority(dir: &std::path::Path) -> Result<(), String> {
    // **The fleet is named FIRST, because the suffix default is built from
    // it.** `<fleet>.home.arpa` gives each fleet its own subdomain of the one
    // namespace that is reserved for exactly this.
    let label = ask("fleet name", DEFAULT_FLEET);

    println!();
    println!("  DNS suffixes this authority may vouch for, comma-separated.");
    println!("  THIS CANNOT BE CHANGED LATER — the constraint is written into the");
    println!("  root, and widening it later means a new root re-trusted on every");
    println!("  browser and device. Name every suffix this fleet may ever be");
    println!("  reached by, including a domain you own but do not serve yet.");
    let suffixes = ask_for_suffixes(&label);
    // **`local` means mDNS is doing the resolving**, and `rivet-mdnsd` already
    // announces `<hostname>.local`. Everything else is a name somebody has to
    // serve, and saying so here costs one line and saves a puzzled hour later.
    for other in suffixes.iter().filter(|s| s.as_str() != DEFAULT_SUFFIX) {
        println!("  note: .{other} needs a DNS server; .local resolves by itself");
    }

    // **The person, which is not the fleet.** This one ends up in front of
    // people rather than in a trust store: `rivet-proxyd`'s index prints it as
    // *"Authenticated as: …"* and the journal records it beside every change
    // this certificate makes. A name with spaces is expected and fine.
    println!();
    println!("  your name, for your own browser certificate. This is printed on");
    println!("  the machine's index page and recorded in the journal beside");
    println!("  everything you change — your own name is a good answer.");
    let operator = ask("operator name", DEFAULT_OPERATOR);
    write_authority(dir, &label, &suffixes, &operator)
}

/// Mint an authority for `label` over `suffixes`, with `operator`'s browser
/// identity, into `dir`.
///
/// # Errors
///
/// A sentence for the screen.
fn write_authority(
    dir: &std::path::Path,
    label: &str,
    suffixes: &[String],
    operator: &str,
) -> Result<(), String> {
    let authority = ca::Authority::root(label, suffixes).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;

    let root = authority.pem().map_err(|e| e.to_string())?;
    // No DNS name: this identifies a person, not a host (`Authority::operator`).
    let browser = authority.operator(operator).map_err(|e| e.to_string())?;

    for (name, body) in [
        ("root.crt", &root.certificate),
        ("root.key", &root.key),
        ("browser.crt", &browser.certificate),
        ("browser.key", &browser.key),
        // **The suffixes are written down rather than derived.** `rcgen` keeps
        // `CertificateParams::from_ca_cert_der` crate-private, so the name
        // constraint cannot be read back out of the certificate — and guessing
        // it from a filename would be a second spelling of the same fact.
        //
        // One per line, because a suffix cannot contain a newline and a person
        // reading this file should be able to see how many there are without
        // knowing a separator.
        ("suffix", &suffixes.join("\n")),
    ] {
        std::fs::write(dir.join(name), body)
            .map_err(|e| format!("{}: {e}", dir.join(name).display()))?;
    }

    println!("  minted {}", dir.display());
    println!("  root.key is the fleet's trust — one copy, and it stays here");
    Ok(())
}

/// Mint a fleet server's certificate for `hostname`.
///
/// # Errors
///
/// A sentence for the screen.
/// **Every name a host may be reached by, in the order the authority permits
/// them** — the list the menu composes, and the list a person types at
/// `ca server`.
///
/// An authority permitted two suffixes is only useful if a server certificate
/// carries both. A leaf for `arbor.local` alone means the authority *could*
/// vouch for `arbor.<fleet>.lan` and the machine still cannot present it, so
/// one way in works and the other shows a browser warning with nothing on
/// screen saying why.
///
/// **The subject is the first**, which is the name a person is most likely to
/// type; nothing authenticates by it (`ADR-028` §4.5) and every name that
/// matters is in the SAN list either way.
///
/// **This is a function so that a test can hold it beside what `ca server`
/// does.** `PLAN.md` §156.2 was two adapters composing the same certificate
/// differently, each self-consistent, and §156.3's conclusion was that only a
/// test spanning both would have caught it.
fn every_name_for(hostname: &str, suffixes: &[String]) -> Vec<String> {
    suffixes
        .iter()
        .map(|s| format!("{hostname}.{s}"))
        .collect()
}

/// `hostname` with one of `suffixes` taken off the end, if it carries one.
///
/// **A person types the name they say out loud.** `rivet-01.local` is the name
/// in the browser and the name in the certificate, so it is what gets typed at
/// `ca server` — and composing on top of it would ask the authority for
/// `rivet-01.local.local`, which the constraint permits and no browser wants.
///
/// Case-insensitive, because DNS is. The longest matching suffix wins, so an
/// authority permitting both `lan` and `home-fleet.lan` strips the one the
/// person actually typed rather than the shorter one it ends with.
fn strip_permitted_suffix(hostname: &str, suffixes: &[String]) -> String {
    let host = hostname.trim_end_matches('.');
    let mut best: Option<&str> = None;
    for suffix in suffixes {
        let suffix = suffix.trim_start_matches('.').trim_end_matches('.');
        if suffix.is_empty() {
            continue;
        }
        let Some(at) = host.len().checked_sub(suffix.len() + 1) else {
            continue;
        };
        let (Some(head), Some(tail)) = (host.get(..at), host.get(at..)) else {
            continue;
        };
        // A leading `.` is what separates the label from the suffix; without it
        // `arivet-01.local` would look like `rivet-01.local` to a byte offset.
        if head.is_empty() || !tail.starts_with('.') {
            continue;
        }
        let Some(rest) = tail.get(1..) else { continue };
        if !rest.eq_ignore_ascii_case(suffix) {
            continue;
        }
        if best.is_none_or(|seen| head.len() < seen.len()) {
            best = Some(head);
        }
    }
    best.unwrap_or(host).to_owned()
}

fn mint_for_host(hostname: String) -> Result<std::path::PathBuf, String> {
    let dir = authority_directory().ok_or_else(|| "no home directory".to_owned())?;
    mint_for_host_in(&dir, &hostname)
}

/// [`mint_for_host`] against the authority in `dir`, so a test can hold the
/// menu's certificate without a home directory.
fn mint_for_host_in(dir: &std::path::Path, hostname: &str) -> Result<std::path::PathBuf, String> {
    let certificate = std::fs::read_to_string(dir.join("root.crt"))
        .map_err(|_| "there is no authority to sign with".to_owned())?;
    let key = std::fs::read_to_string(dir.join("root.key"))
        .map_err(|_| "the authority has no key here".to_owned())?;
    // **Read from the certificate, not from the note beside it.** Ruled
    // 2026-09-06 with Q13, and it removes a whole class of failure rather than
    // guarding against it: this used to read a `suffix` file and, when that
    // file was missing, PROMPT for the list — a prompt with no command-line
    // equivalent, defaulting to `local` alone. A scripted run took the default,
    // recorded one suffix where the authority permitted three, and every
    // certificate minted afterwards was short by two names with nothing
    // reporting it.
    //
    // The certificate cannot drift from itself. `ADR-011` §3.
    let suffixes = ca::permitted_suffixes(&certificate).map_err(|e| e.to_string())?;
    if suffixes.is_empty() {
        return Err(
            "this authority constrains no DNS names, so it cannot vouch for a host".to_owned(),
        );
    }

    let names = every_name_for(hostname, &suffixes);
    let full = names[0].clone();
    let minted =
        ca::issue_from(&certificate, &key, &names, &full).map_err(|e| e.to_string())?;
    let path = dir.join(format!("server-{full}.crt"));
    std::fs::write(&path, &minted.certificate).map_err(|e| format!("{}: {e}", path.display()))?;
    let key_path = dir.join(format!("server-{full}.key"));
    std::fs::write(&key_path, &minted.key).map_err(|e| format!("{}: {e}", key_path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    // -----------------------------------------------------------------------
    // resolve_rivet_root — RIVET_AT's own decision, pure: no environment in
    // the loop, so `authority_directory`/`save::directory`/`default_identity`
    // all agreeing with `cargo rivet provision`'s own `--at` is provable with
    // two `OsString`s and nothing else. See this module's own `RIVET_AT`
    // section for the defect this closes.
    // -----------------------------------------------------------------------

    /// **`RIVET_AT` wins over `HOME`**, whether or not `HOME` is also set —
    /// this is the whole fix: `cargo rivet provision --at DIR` sets both,
    /// inheriting this process's real `HOME` alongside its own explicit
    /// `RIVET_AT`, and `RIVET_AT` must be the one that decides.
    #[test]
    fn rivet_at_wins_over_home() {
        let root = resolve_rivet_root(
            Some(std::ffi::OsString::from("D:\\scratch\\at")),
            Some(std::ffi::OsString::from("D:\\real\\home")),
        );
        assert_eq!(root, Some(std::path::PathBuf::from("D:\\scratch\\at")));
    }

    /// **`RIVET_AT` unset falls back to `$HOME/.rivet`** — this tool's own
    /// default, run directly with nobody setting the variable.
    #[test]
    fn no_rivet_at_falls_back_to_home_dot_rivet() {
        let root = resolve_rivet_root(None, Some(std::ffi::OsString::from("D:\\real\\home")));
        assert_eq!(root, Some(std::path::PathBuf::from("D:\\real\\home").join(".rivet")));
    }

    /// **An empty `RIVET_AT` is treated as unset** — `var_os` answers
    /// `Some("")` for a variable a shell spells *unset*, on
    /// `credentials.rs::set_to_something`'s own footing.
    #[test]
    fn an_empty_rivet_at_is_treated_as_unset() {
        let root = resolve_rivet_root(
            Some(std::ffi::OsString::new()),
            Some(std::ffi::OsString::from("D:\\real\\home")),
        );
        assert_eq!(root, Some(std::path::PathBuf::from("D:\\real\\home").join(".rivet")));
    }

    /// Neither set is `None`, the same refusal shape every caller here
    /// already handles (a cold container with no home at all).
    #[test]
    fn neither_set_resolves_to_nothing() {
        assert_eq!(resolve_rivet_root(None, None), None);
    }

    /// A real directory under the OS temporary directory, removed at the end
    /// — `cmd/rivet-fleet/src/main.rs`'s own `Scratch`, copied rather than
    /// shared: this crate has no `[lib]` target for a `#[cfg(test)]` helper
    /// to live in. Used below for a scratch staging directory and a scratch
    /// fleet repository's `manifest.toml` (`resolve_carry`'s own tests).
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("rivet-provision-fleet-repo-{label}-{n}"));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("a scratch directory");
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A person typing two suffixes types them the way they think of them, and
    /// being refused over a space is the friction that makes somebody guess
    /// instead of read.
    #[test]
    fn suffixes_may_be_separated_however_a_person_typed_them() {
        for typed in [
            "local,lindell.house",
            "local, lindell.house",
            "local lindell.house",
            "  local ,  lindell.house  ",
            "local,lindell.house,",
        ] {
            assert_eq!(
                split_suffixes(typed),
                vec!["local".to_owned(), "lindell.house".to_owned()],
                "{typed:?}"
            );
        }
    }

    /// **An empty answer produces an empty list, not a list containing
    /// nothing.** The distinction matters one call later: `Authority::root`
    /// refuses both, and a caller that had silently turned "" into `[""]` would
    /// be relying on that refusal to catch its own bug.
    #[test]
    fn nothing_typed_is_no_suffixes_at_all() {
        for typed in ["", "   ", ",", " , , "] {
            assert!(split_suffixes(typed).is_empty(), "{typed:?}");
        }
    }

    /// The two names a person is offered are the role and the fleet, and they
    /// are not the tool's own name. Ruled 2026-09-06.
    #[test]
    fn the_defaults_are_the_ones_the_maintainer_ruled() {
        assert_eq!(DEFAULT_OPERATOR, "fleet-owner");
        assert_eq!(DEFAULT_FLEET, "my-fleet");
        assert_eq!(DEFAULT_SUFFIX, "local");
    }

    /// **What a person is offered when minting**: what Rivet serves today, and
    /// the one namespace that can never be delegated. Ruled 2026-09-06.
    #[test]
    fn the_offered_suffixes_are_mdns_and_the_reserved_namespace() {
        assert_eq!(
            default_suffixes("my-fleet"),
            vec!["local".to_owned(), "my-fleet.home.arpa".to_owned()]
        );
    }

    /// **`.lan` is never offered**, and this is the test that says so rather
    /// than a comment somebody edits away. It is undelegated rather than
    /// reserved, so a root constrained to it could one day vouch for real
    /// names — on the one field that cannot be re-cut.
    #[test]
    fn no_default_offers_a_suffix_that_could_one_day_be_delegated() {
        for fleet in ["my-fleet", "lan", "home", "Someone's fleet"] {
            for offered in default_suffixes(fleet) {
                assert!(
                    offered == DEFAULT_SUFFIX || offered.ends_with(HOME_ARPA),
                    "{fleet:?} offered {offered:?}, which is neither mDNS nor RFC 8375"
                );
            }
        }
    }

    /// A fleet name is free text and a DNS label is not, so the label is
    /// derived rather than assumed. A name that cannot produce one leaves the
    /// default at mDNS alone rather than composing a suffix the authority would
    /// then refuse.
    #[test]
    fn a_fleet_name_becomes_a_label_or_it_becomes_nothing() {
        assert_eq!(dns_label("my-fleet").as_deref(), Some("my-fleet"));
        assert_eq!(dns_label("Someone's fleet").as_deref(), Some("someone-s-fleet"));
        assert_eq!(dns_label("  Rivet  Fleet  ").as_deref(), Some("rivet-fleet"));
        assert_eq!(dns_label("--"), None);
        assert_eq!(dns_label(""), None);
        assert_eq!(dns_label("!!!"), None);

        assert_eq!(default_suffixes("!!!"), vec!["local".to_owned()]);

        // A label may not exceed 63 bytes, and may not end in a hyphen after
        // the cut.
        let long = dns_label(&"a".repeat(100)).expect("a label");
        assert_eq!(long.len(), 63);
        assert!(!long.ends_with('-'));
    }

    /// **The two adapters ask for the SAME certificate.**
    ///
    /// This is the assertion `PLAN.md` §156.2 did not have. The menu composes
    /// a name list from the authority; the command line takes one typed. Both
    /// were self-consistent and they produced different certificates for months
    /// — `ca server arbor` issued a leaf whose only name was `DNS:arbor`, which
    /// no browser matches, while the menu issued three.
    ///
    /// `ADR-006` §3.1: *"an operation that exists in only one interface is a
    /// defect in that interface"* — and an operation that exists in both while
    /// producing different results is the same defect wearing a disguise.
    #[test]
    fn the_menu_and_the_command_line_compose_the_same_names() {
        let suffixes = vec![
            "local".to_owned(),
            "home-fleet.home.arpa".to_owned(),
            "home-fleet.lan".to_owned(),
        ];

        // What the menu composes, given the authority's own constraint.
        let menu = every_name_for("arbor", &suffixes);

        // What a person types at `ca server`, through the parser that verb
        // uses. Written the way a person writes it — one with a space, because
        // `split_suffixes` exists to tolerate that.
        let typed = split_suffixes("arbor.local, arbor.home-fleet.home.arpa,arbor.home-fleet.lan");

        assert_eq!(menu, typed, "the two interfaces do not ask for the same certificate");
        assert_eq!(menu.len(), suffixes.len(), "a suffix was dropped on the way");
    }

    /// **`ADR-059` §8, 2026-09-22: an untouched Fleet question still means no
    /// role**, even though `"stand-alone"` is no longer one of its offered
    /// choices. Without this, `Field::choice`'s generic "the first is the
    /// default" rule would make a `w` where the operator never opened this
    /// question silently provision a fleet server — see the panic this test
    /// stands in for, caught by `even_a_stand_alone_machine_gets_the_table_
    /// written_out` when `FleetRole::ALL` first dropped the word.
    #[test]
    fn an_untouched_fleet_question_still_defaults_to_no_role() {
        let fleet = questions().into_iter().find(|f| f.label == "Fleet").expect("Fleet exists");
        assert_eq!(fleet.value, "stand-alone");
        assert_eq!(fleet.default, "stand-alone");
        assert_eq!(fleet.state(), crate::screen::State::Default);
        // The two words actually offered are the live ones — the pre-filled
        // value is not one of them, and that is the point.
        assert_eq!(fleet.kind, Kind::Choice(vec!["server".to_owned(), "member".to_owned()]));
    }

    /// **The old hand-written `questions()`'s own order, captured here
    /// before that function was deleted** — `docs/PLAN-Feature-Set-2026-09-12.md`
    /// §5.2 item A: *"day one reproduces today's order and question numbers
    /// exactly."* `rivet_config::interview_tests` pins the schema-level
    /// sequence this is generated from; this is the same claim at the one
    /// remove that module cannot see — `questions()` appends `Identity`,
    /// which has no schema key. `Nameservers` is tenth and last, so the nine
    /// numbers every note and rehearsal script types are the nine they were.
    #[test]
    fn the_generated_menu_keeps_the_old_nine_in_order_and_adds_a_tenth() {
        let labels: Vec<&str> = questions().iter().map(|f| f.label).collect();
        assert_eq!(
            labels,
            [
                "Hostname", "Addressing", "Address", "Gateway", "Mode", "Fleet", "Timezone",
                "Auto-run", "Identity", NAMESERVERS,
            ]
        );
    }

    // ---- Nameservers: `[network.resolver]` for a static address ----------

    /// The answers of a static machine as the menu leaves them: the address
    /// typed, the gateway it implied accepted, and the Nameservers default
    /// followed from it exactly as the edit branch follows it.
    fn answered_static(extra: &[(&str, &str)]) -> Vec<Field> {
        let mut pairs = vec![
            ("Hostname", "arbor"),
            ("Addressing", "static"),
            ("Address", "192.0.2.10/24"),
            ("Gateway", "192.0.2.1"),
            ("Timezone", "UTC"),
        ];
        pairs.extend_from_slice(extra);
        let mut fields = answered(&pairs);
        follow_gateway(&mut fields);
        fields
    }

    /// **A static answer composes a `[network.resolver]` the board's own
    /// parser accepts**, with the nameservers in the order they were typed.
    #[test]
    fn a_static_answer_composes_a_resolver_the_parser_accepts() {
        let mut fields = answered_static(&[]);
        let f = fields.iter_mut().find(|f| f.label == NAMESERVERS).expect("the field");
        f.set("192.0.2.53, 1.1.1.1 9.9.9.9", field::nameservers);
        assert!(f.error.is_none(), "{:?}", f.error);

        let doc = compose(&fields, "arbor.local", TEST_REPOSITORY_ID).expect("composes");
        assert!(
            doc.contains("\n[network.resolver]\nnameservers = [\"192.0.2.53\", \"1.1.1.1\", \"9.9.9.9\"]\n"),
            "{doc}"
        );
        let parsed = rivet_config::Document::parse(&doc).expect("the board's parser accepts it");
        let asked: Vec<String> = parsed.nameservers().iter().map(ToString::to_string).collect();
        assert_eq!(asked, ["192.0.2.53", "1.1.1.1", "9.9.9.9"], "{doc}");
        // The reserved sub-table is not read as a second interface.
        assert_eq!(parsed.network().len(), 1, "{doc}");
    }

    /// **The default is the gateway**: untouched, the field holds it and a
    /// bare Return at its prompt takes it, and it composes into the document.
    /// A typed answer is not overwritten when the gateway changes afterwards.
    #[test]
    fn the_nameservers_default_is_the_gateway() {
        let mut fields = answered_static(&[]);
        let ns = fields.iter().find(|f| f.label == NAMESERVERS).expect("the field");
        assert_eq!(ns.default, "192.0.2.1");
        assert_eq!(ns.value, "192.0.2.1");
        assert_eq!(ns.state(), crate::screen::State::Default, "a default is dim, not an answer");
        assert_eq!(chosen_answer(ns, "", &[]), "192.0.2.1", "a bare Return takes the gateway");

        let doc = compose(&fields, "arbor.local", TEST_REPOSITORY_ID).expect("composes");
        let parsed = rivet_config::Document::parse(&doc).expect("parses");
        let asked: Vec<String> = parsed.nameservers().iter().map(ToString::to_string).collect();
        assert_eq!(asked, ["192.0.2.1"], "{doc}");

        // A gateway changed later carries an untouched default with it...
        let g = fields.iter_mut().find(|f| f.label == "Gateway").expect("Gateway");
        g.set("192.0.2.254", field::ipv4);
        follow_gateway(&mut fields);
        assert_eq!(value_of(&fields, NAMESERVERS), "192.0.2.254");

        // ...and never a typed answer.
        let ns = fields.iter_mut().find(|f| f.label == NAMESERVERS).expect("the field");
        ns.set("1.1.1.1", field::nameservers);
        let g = fields.iter_mut().find(|f| f.label == "Gateway").expect("Gateway");
        g.set("192.0.2.1", field::ipv4);
        follow_gateway(&mut fields);
        assert_eq!(value_of(&fields, NAMESERVERS), "1.1.1.1");
        let ns = fields.iter().find(|f| f.label == NAMESERVERS).expect("the field");
        assert_eq!(ns.default, "192.0.2.1", "the default still follows");
    }

    /// A minimal document naming `list` as its nameservers, for the board's
    /// parser to rule on — and three well-formed ones parse, so a refusal of
    /// this shape is about the list and nothing else in it.
    fn resolver_document(list: &[&str]) -> String {
        let quoted: Vec<String> = list.iter().map(|one| format!("\"{one}\"")).collect();
        format!("version = 1\n\n[network.resolver]\nnameservers = [{}]\n", quoted.join(", "))
    }

    /// **Four nameservers are refused at the prompt, and the parser refuses
    /// the same four** — the two refusals are held side by side so this check
    /// cannot drift from the one the board makes.
    #[test]
    fn four_nameservers_are_refused() {
        let four = "192.0.2.1, 192.0.2.2, 192.0.2.3, 192.0.2.4";
        let why = field::nameservers(four).expect_err("four were accepted");
        assert!(why.contains("4 given") && why.contains("at most 3"), "{why}");
        assert!(field::nameservers("192.0.2.1, 192.0.2.2, 192.0.2.3").is_ok());

        let three = ["192.0.2.1", "192.0.2.2", "192.0.2.3"];
        assert!(rivet_config::Document::parse(&resolver_document(&three)).is_ok(), "the control");
        let board = rivet_config::Document::parse(&resolver_document(&[
            "192.0.2.1", "192.0.2.2", "192.0.2.3", "192.0.2.4",
        ]));
        assert!(board.is_err(), "the parser accepted four: {board:?}");

        // And `compose` will not write them past a refused field either.
        let mut fields = answered_static(&[]);
        let f = fields.iter_mut().find(|f| f.label == NAMESERVERS).expect("the field");
        f.set(four, field::nameservers);
        let refused = compose(&fields, "arbor.local", TEST_REPOSITORY_ID).expect_err("composed four");
        assert!(refused.contains(NAMESERVERS), "{refused}");
    }

    /// **An answer that is not an IPv4 address is refused, by the entry that
    /// is not one** — an IPv6 address, a name, an octet out of range, a
    /// prefix.
    #[test]
    fn a_non_ipv4_answer_is_refused() {
        for (typed, named) in [
            ("2606:4700:4700::1111", "2606:4700:4700::1111"),
            ("192.0.2.1, dns.example", "dns.example"),
            ("300.1.1.1", "300.1.1.1"),
            ("192.0.2.1/24", "192.0.2.1/24"),
        ] {
            let why = field::nameservers(typed).expect_err(typed);
            assert!(why.contains(named) && why.contains("not an IPv4 address"), "{typed}: {why}");
            assert!(
                rivet_config::Document::parse(&resolver_document(&[named])).is_err(),
                "the parser accepted {named}"
            );
        }
        assert!(rivet_config::Document::parse(&resolver_document(&["192.0.2.1"])).is_ok(), "the control");
        // Nothing at all is this tool's own refusal, not the parser's.
        assert!(field::nameservers("").is_err());
        assert!(field::nameservers(" , ").is_err());
    }

    /// **DHCP composes no `[network.resolver]`, and the question is not
    /// asked** — the lease supplies the resolver, and a table would override
    /// it. A value left behind by switching from static is not composed.
    #[test]
    fn dhcp_composes_no_resolver_table() {
        let mut fields = answered(&[
            ("Hostname", "arbor"),
            ("Gateway", "192.0.2.1"),
            ("Timezone", "UTC"),
            (NAMESERVERS, "1.1.1.1"),
        ]);
        assert_eq!(value_of(&fields, "Addressing"), "dhcp");
        assert!(!asked(&fields, NAMESERVERS), "a dhcp machine is asked no nameservers");
        assert!(asked(&fields, "Gateway") && asked(&fields, "Identity"), "only Nameservers is gated");

        let doc = compose(&fields, "arbor.local", TEST_REPOSITORY_ID).expect("composes");
        assert!(!doc.contains("resolver"), "{doc}");
        assert!(rivet_config::Document::parse(&doc).expect("parses").resolver().is_none());

        // A refused, hidden answer does not block a dhcp document either.
        let f = fields.iter_mut().find(|f| f.label == NAMESERVERS).expect("the field");
        f.set("not an address", field::nameservers);
        assert!(compose(&fields, "arbor.local", TEST_REPOSITORY_ID).is_ok());

        let a = fields.iter_mut().find(|f| f.label == "Addressing").expect("Addressing");
        a.set("static", field::anything);
        assert!(asked(&fields, NAMESERVERS), "a static machine is asked");
    }

    /// **The answer is saved in the notebook and a re-run preselects it**,
    /// and a notebook from before the field existed follows its gateway.
    #[test]
    fn the_nameservers_answer_is_saved_and_resumed() {
        let mut fields = answered_static(&[]);
        let f = fields.iter_mut().find(|f| f.label == NAMESERVERS).expect("the field");
        f.set("192.0.2.53, 1.1.1.1", field::nameservers);
        let notebook = save::render(&fields);
        assert!(notebook.contains("nameservers = \"192.0.2.53, 1.1.1.1\""), "{notebook}");

        let mut resumed = questions();
        save::apply(&notebook, &mut resumed, checker);
        follow_gateway(&mut resumed);
        assert_eq!(value_of(&resumed, NAMESERVERS), "192.0.2.53, 1.1.1.1");

        let older: String = notebook.lines().filter(|l| !l.starts_with("nameservers")).map(|l| format!("{l}\n")).collect();
        let mut resumed = questions();
        save::apply(&older, &mut resumed, checker);
        follow_gateway(&mut resumed);
        assert_eq!(value_of(&resumed, NAMESERVERS), "192.0.2.1");
    }

    /// Answer the menu without a menu, so `compose` can be held to its output.
    fn answered(pairs: &[(&str, &str)]) -> Vec<Field> {
        let mut fields = questions();
        for (label, value) in pairs {
            let Some(f) = fields.iter_mut().find(|f| &f.label == label) else {
                panic!("{label} is not a field this menu has");
            };
            let check = checker(f.label);
            f.set(value, check);
        }
        fields
    }

    /// Advice from before `ADR-054` §2, when provisioning built its own card.
    const CARD_BUILDING: [&str; 3] = ["build.sh", "RIVET_IDENTITY", "rivet-image --identity"];

    /// **Under `cargo rivet provision`, nothing the interview prints after `w`
    /// tells a person to build a card.** Found on the maintainer's first real
    /// run, 2026-09-14: it did, moments before the orchestrator wrote one.
    #[test]
    fn under_the_orchestrator_nothing_says_to_build_a_card() {
        let identity = "C:\\scratch\\identity\\board";
        let mut said = String::new();
        for fleet in ["stand-alone", "server", "member"] {
            let fields = answered(&[
                ("Hostname", "lounge"),
                ("Mode", "kiosk"),
                ("Fleet", fleet),
                ("Auto-run", "photo-frame"),
                ("Identity", identity),
            ]);
            said.push_str(&what_the_role_decided(&fields, true));
        }
        assert!(said.contains("cargo rivet provision puts the Identity"), "{said}");
        let document = std::path::Path::new("lounge.provisioning.toml");
        assert_eq!(after_staging(true, document, identity), None);
        for stale in CARD_BUILDING {
            assert!(!said.contains(stale), "{stale}:\n{said}");
        }
        // The Identity question's own `?` text is printed on either route.
        let explain = questions().iter().find(|f| f.label == "Identity").map(|f| f.explain).unwrap_or_default();
        for stale in CARD_BUILDING {
            assert!(!explain.contains(stale), "{stale}: {explain}");
        }
    }

    /// **Run directly, it names the command that writes a card** — never a
    /// build.
    #[test]
    fn standalone_the_card_is_written_by_cargo_rivet_provision() {
        let fields = answered(&[("Hostname", "arbor"), ("Fleet", "server")]);
        let said = what_the_role_decided(&fields, false);
        let document = std::path::Path::new("arbor.provisioning.toml");
        let next = after_staging(false, document, "iddir").expect("a next step");
        assert_eq!(next, "  write the card with:  cargo rivet provision --document arbor.provisioning.toml --identity iddir");
        for stale in CARD_BUILDING {
            assert!(!said.contains(stale), "{stale}:\n{said}");
            assert!(!next.contains(stale), "{stale}: {next}");
        }
    }

    /// **An auto-run the system image carries is not warned about**; an
    /// application's still is. `rivet-launcher` is an image service
    /// (`rivet_core::service::IMAGE_SERVICE_NAMES`), so it cannot be missing.
    #[test]
    fn the_install_warning_is_for_applications_not_image_services() {
        let kiosk = |auto_run| answered(&[("Hostname", "lounge"), ("Mode", "kiosk"), ("Auto-run", auto_run)]);
        for orchestrated in [true, false] {
            let launcher = what_the_role_decided(&kiosk("rivet-launcher"), orchestrated);
            assert!(!launcher.contains("NOTHING HERE CAN CHECK"), "{launcher}");
            assert!(launcher.contains("the system image itself carries"), "{launcher}");
            let application = what_the_role_decided(&kiosk("photo-frame"), orchestrated);
            assert!(application.contains("NOTHING HERE CAN CHECK THAT IT WILL BE INSTALLED"), "{application}");
        }
    }

    /// **The bytes `w` actually wrote, held against the board's parser.**
    ///
    /// This literal is not a reconstruction: it is `arbor.provisioning.toml` as
    /// `tools/provision/rehearse-menu.sh` leaves it, the run that verifies
    /// `SPEC-05` §4. The harness asserts the tool *said* it parsed; this asserts
    /// the file independently, so the two claims do not rest on one code path.
    ///
    /// **And it is asserted twice over** — as a literal, and as the thing
    /// `compose` produces from the same answers. A literal alone would pin
    /// bytes nothing produces any more; `compose` alone would be the tool
    /// agreeing with itself.
    #[test]
    fn the_bytes_the_menu_wrote_for_a_fleet_server_are_a_document() {
        let written = "\
version = 1
hostname = \"arbor\"
timezone = \"America/Chicago\"

[network.eth0]
method = \"dhcp\"

[control-plane]
listen = \"0.0.0.0:7777\"

[applications.rivet-proxyd]
upstream-name = \"arbor.local\"

[services.rivet-fleetd]
enabled = true

[services.rivet-mdnsd]
enabled = true

[services.rivet-proxyd]
enabled = true

[repos.fleet]
id = \"0123456789abcdef\"
";
        let composed = compose(
            &answered(&[
                ("Hostname", "arbor"),
                ("Fleet", "server"),
                ("Timezone", "America/Chicago"),
            ]),
            "arbor.local",
            TEST_REPOSITORY_ID,
        )
        .expect("the menu's own answers must compose");
        assert_eq!(composed, written, "the menu no longer writes the pinned bytes");

        let parsed = rivet_config::Document::parse(written)
            .expect("the file rivet-provision wrote must be a document the board accepts");
        assert_eq!(parsed.hostname().map(ToString::to_string).as_deref(), Some("arbor"));
        assert_eq!(parsed.network().len(), 1);

        // **The three defaults of the ROLE**, each of which was absent on
        // 2026-09-07 and each of which cost a diagnosis cycle.
        assert_eq!(
            parsed.effective_control_plane().listen(),
            "0.0.0.0:7777",
            "a fleet server that listens on loopback is a board nobody can reach"
        );
        for service in ["rivet-fleetd", "rivet-proxyd", "rivet-mdnsd"] {
            assert!(
                parsed.service(service).is_some_and(rivet_config::services::Service::enabled),
                "a fleet server's document must enable {service}"
            );
        }
        // **The zone survived into the document**, which is the whole of the
        // timezone requirement: `/etc/localtime` is projected from this key
        // before any service starts, and a machine without it keeps UTC
        // silently.
        assert_eq!(
            parsed.timezone().map(ToString::to_string).as_deref(),
            Some("America/Chicago")
        );
        // **And the repository it serves** (`.rulings/` R-184 D3): the
        // board's own parser reads `[repos.fleet]` with the id it was given.
        let repos = parsed.repos();
        assert_eq!(repos.len(), 1, "a fleet server declares exactly one repository");
        assert_eq!(repos[0].name().as_str(), rivetos_fleet_repo::SERVED_NAME);
        assert_eq!(repos[0].id().as_str(), TEST_REPOSITORY_ID);
    }

    /// A fixed, well-formed identifier, so a composed document's bytes are
    /// comparable. The real `w` key mints one with [`mint_repository_id`].
    const TEST_REPOSITORY_ID: &str = "0123456789abcdef";

    /// **Only a server declares `[repos.fleet]`** (`.rulings/` R-184 D3). A
    /// member and a stand-alone machine serve no fleet repository, and a
    /// repository declared on one would be created on its disk by every boot
    /// for nothing.
    #[test]
    fn only_a_fleet_server_declares_the_fleet_repository() {
        for (fleet, expected) in [("server", 1usize), ("member", 0), ("stand-alone", 0)] {
            let doc = compose(
                &answered(&[("Hostname", "arbor"), ("Fleet", fleet), ("Timezone", "UTC")]),
                "arbor.local",
                TEST_REPOSITORY_ID,
            )
            .unwrap_or_else(|e| panic!("{fleet}: {e}"));
            let parsed = rivet_config::Document::parse(&doc).expect("parses");
            assert_eq!(parsed.repos().len(), expected, "{fleet}:\n{doc}");
            assert_eq!(doc.contains("[repos."), expected == 1, "{fleet}:\n{doc}");
        }
    }

    /// **A minted identifier is one the board accepts, and two mints differ.**
    /// Not a statistical claim about collisions — a check that the grammar is
    /// `rivet_config::repos::RepoId`'s and that nothing is constant.
    #[test]
    fn a_minted_repository_id_is_well_formed_and_not_a_constant() {
        let first = mint_repository_id();
        let second = mint_repository_id();
        assert!(rivet_config::repos::RepoId::new(&first).is_some(), "{first}");
        assert!(rivet_config::repos::RepoId::new(&second).is_some(), "{second}");
        assert_ne!(first, second, "two mints produced the same identifier");
    }

    /// **A malformed identifier is refused before a server document is
    /// composed**, rather than written and refused by the board on first boot.
    #[test]
    fn a_server_with_a_malformed_repository_id_is_refused() {
        let refused = compose(
            &answered(&[("Hostname", "arbor"), ("Fleet", "server"), ("Timezone", "UTC")]),
            "arbor.local",
            "not-an-id",
        )
        .expect_err("a malformed identifier must not compose");
        assert!(refused.contains("not-an-id"), "{refused}");
        assert!(refused.contains("16 lowercase hexadecimal"), "{refused}");
    }

    /// **A fleet member's document enables `rivet-proxyd` too, and its card
    /// carries nothing** — `ADR-050` §5's role table, `.rulings/` R-159 R21,
    /// and `ADR-060`: the proxy is part of the image, so enabling it is the
    /// whole of what a member's role asks for.
    #[test]
    fn a_members_document_enables_the_proxy_and_carries_nothing() {
        let doc = compose(
            &answered(&[
                ("Hostname", "kitchen-pi"),
                ("Fleet", "member"),
                ("Timezone", "America/Chicago"),
            ]),
            "kitchen-pi.local",
            TEST_REPOSITORY_ID,
        )
        .expect("a member's answers must compose");
        let parsed = rivet_config::Document::parse(&doc)
            .expect("the file rivet-provision wrote must be a document the board accepts");

        for service in ["rivet-proxyd", "rivet-mdnsd"] {
            assert!(
                parsed.service(service).is_some_and(rivet_config::services::Service::enabled),
                "a fleet member's document must enable {service}:\n{doc}"
            );
        }
        assert!(
            parsed.service("rivet-fleetd").is_none(),
            "a fleet member does not run fleetd:\n{doc}"
        );

        // Both are image services and ship in every card; each is switched on
        // and neither is CARRIED.
        assert!(parsed.carried_services().is_empty(), "{doc}");
    }

    // ---- R-178 P3: resolve_carry / carried_version_from_answer -----------

    /// **Newest preselected** — R-176 Q6's own words. A blank answer takes
    /// `versions[0]`, which the caller has already sorted newest first.
    #[test]
    fn a_blank_answer_carries_the_newest_version() {
        let versions = vec!["0.2.0".to_owned(), "0.1.10".to_owned(), "0.1.0".to_owned()];
        assert_eq!(
            carried_version_from_answer("rivet-fleetd", &versions, ""),
            Ok("0.2.0".to_owned())
        );
    }

    /// **Pick an older one** — by number, or by typing the version back.
    #[test]
    fn a_typed_answer_picks_an_older_version_by_number_or_by_name() {
        let versions = vec!["0.2.0".to_owned(), "0.1.10".to_owned(), "0.1.0".to_owned()];
        assert_eq!(
            carried_version_from_answer("rivet-fleetd", &versions, "3"),
            Ok("0.1.0".to_owned())
        );
        assert_eq!(
            carried_version_from_answer("rivet-fleetd", &versions, "0.1.10"),
            Ok("0.1.10".to_owned())
        );
    }

    /// A number out of range, or text matching nothing published, is refused
    /// by name rather than silently substituting the default.
    #[test]
    fn an_answer_matching_nothing_is_refused() {
        let versions = vec!["0.2.0".to_owned()];
        let err = carried_version_from_answer("rivet-fleetd", &versions, "9")
            .expect_err("out of range");
        assert!(err.contains("not between 1 and 1"), "{err}");
        let err = carried_version_from_answer("rivet-fleetd", &versions, "0.9.9")
            .expect_err("never published");
        assert!(err.contains("0.9.9"), "{err}");
        assert!(err.contains("rivet-fleetd"), "{err}");
    }

    /// **`x`/`X` leaves without choosing, refused with its own message
    /// rather than answered as a version.** This is the parsing half, pure
    /// and tested with no stdin; `choose_carried_version` itself (the I/O
    /// half) stays untested, like every other menu here.
    #[test]
    fn x_or_capital_x_refuses_cleanly_instead_of_failing_to_parse() {
        let versions = vec!["0.2.0".to_owned(), "0.1.10".to_owned()];
        for typed in ["x", "X"] {
            let err = carried_version_from_answer("rivet-fleetd", &versions, typed)
                .expect_err("x must not be treated as a chosen version");
            assert!(err.contains("rivet-fleetd"), "{err}");
            assert!(
                !err.contains("is not one of the published versions"),
                "x must not be refused as an unrecognised answer: {err}"
            );
        }
    }

    /// A scratch fleet repository holding one `manifest.toml`, built by hand
    /// so `resolve_carry`'s own tests do not need a real `cargo rivet fleet`.
    fn manifest_with(entries: &[(&str, &str, &str)]) -> Scratch {
        let scratch = Scratch::new("resolve-carry");
        let mut text = String::new();
        for (product, version, kind) in entries {
            text.push_str(&format!(
                "\n[[artifact]]\ndigest = \"{}\"\nproduct = \"{product}\"\nversion = \"{version}\"\n\
                 arch = \"aarch64\"\nkind = \"{kind}\"\nsize = 30\nadded = \"2026-09-13\"\n",
                "a".repeat(64),
            ));
        }
        std::fs::write(scratch.0.join("manifest.toml"), text).expect("write the manifest");
        scratch
    }

    /// **A document that carries nothing needs no repository at all** — a
    /// stand-alone machine, or a server/member answered with `root = None`
    /// should still pass, since [`Document::carried_services`] is empty.
    #[test]
    fn a_document_with_nothing_to_carry_is_returned_unchanged_even_with_no_repository() {
        let doc = "version = 1\nhostname = \"arbor\"\n".to_owned();
        assert_eq!(resolve_carry(doc.clone(), None), Ok(doc));
    }

    /// **No fleet repository at all, and something must be carried, is a
    /// named refusal** — nothing is written.
    #[test]
    fn a_server_document_with_no_repository_root_is_refused() {
        let doc = "version = 1\nhostname = \"arbor\"\n\n[services.rivet-fleetd]\nenabled = true\n"
            .to_owned();
        let err = resolve_carry(doc, None).expect_err("no repository at all");
        assert!(err.contains("NOTHING WAS WRITTEN"), "{err}");
        assert!(err.contains("rivet-fleetd"), "{err}");
    }

    /// **A required product with nothing published is a named refusal** —
    /// R-178 P3's own words, naming `cargo rivet fleet publish`.
    #[test]
    fn a_required_product_with_no_published_version_is_refused() {
        let scratch = manifest_with(&[]);
        let doc = "version = 1\nhostname = \"arbor\"\n\n[services.rivet-fleetd]\nenabled = true\n"
            .to_owned();
        let err = resolve_carry(doc, Some(&scratch.0)).expect_err("nothing published");
        assert!(err.contains("NOTHING WAS WRITTEN"), "{err}");
        assert!(err.contains("rivet-fleetd"), "{err}");
        assert!(err.contains("cargo rivet fleet publish"), "{err}");
    }

    /// **`timezone` is written before the first table header, or it is not a
    /// top-level key at all.**
    ///
    /// TOML puts a bare key inside whichever table precedes it, so a `timezone`
    /// line emitted after `[network.eth0]` would silently become
    /// `network.eth0.timezone` and be refused as an unknown key — and appending
    /// one to a finished document is exactly what `aura-controller`'s General
    /// page tells a person to do.
    #[test]
    fn the_timezone_is_a_top_level_key_and_precedes_every_table() {
        let doc = compose(
            &answered(&[
                ("Hostname", "arbor"),
                ("Fleet", "server"),
                ("Timezone", "Europe/Berlin"),
            ]),
            "arbor.local",
            TEST_REPOSITORY_ID,
        )
        .expect("composes");
        let zone_at = doc.find("timezone =").expect("the document names a zone");
        let first_table = doc.find('[').expect("the document has a table");
        assert!(zone_at < first_table, "timezone is written inside a table:\n{doc}");
    }

    /// **A machine somebody else administers binds every interface; one nobody
    /// does keeps loopback** — `ADR-004` §6, its role-defaults section.
    ///
    /// The loopback value is read from `rivet_config` rather than typed, so
    /// there is one spelling of the schema's default (`ADR-011` §3).
    #[test]
    fn the_listener_is_decided_by_who_administers_the_machine() {
        assert_eq!(listen_for("server"), "0.0.0.0:7777");
        assert_eq!(listen_for("member"), "0.0.0.0:7777");
        assert_eq!(listen_for("stand-alone"), rivet_config::control_plane::DEFAULT_LISTEN);
        assert_eq!(REACHABLE_LISTEN, "0.0.0.0:7777");
        // The value that produced the unreachable board is still the SCHEMA's
        // default, and this test says so rather than leaving it looking like an
        // oversight: a machine that has lost its document must not start
        // answering the network because a file went missing.
        assert_eq!(rivet_config::control_plane::DEFAULT_LISTEN, "127.0.0.1:7777");
    }

    /// **Every composed document carries a `[control-plane]` table**, whatever
    /// the address in it, because an absent table cannot be edited on a machine
    /// whose control plane is the thing being repaired.
    #[test]
    fn even_a_stand_alone_machine_gets_the_table_written_out() {
        let doc = compose(
            &answered(&[("Hostname", "bench"), ("Timezone", "UTC")]),
            "bench.local",
            TEST_REPOSITORY_ID,
        )
        .expect("composes");
        assert!(doc.contains("[control-plane]"), "{doc}");
        assert!(doc.contains("listen = \"127.0.0.1:7777\""), "{doc}");
        let parsed = rivet_config::Document::parse(&doc).expect("parses");
        // Stated, not defaulted: the operator's document says where it listens.
        assert!(parsed.control_plane().is_some());
        assert!(parsed.services().is_empty(), "a stand-alone machine turns nothing on");
    }

    /// **A kiosk gets its table, and the application it names gets enabled.**
    ///
    /// An `auto-run` naming an application the document does not enable is the
    /// same black screen as one naming an application that is absent — the
    /// supervisor starts nothing the document has not enabled.
    #[test]
    fn a_kiosk_names_the_application_that_takes_the_screen_and_enables_it() {
        let doc = compose(
            &answered(&[
                ("Hostname", "lounge"),
                ("Mode", "kiosk"),
                ("Fleet", "member"),
                ("Timezone", "America/Chicago"),
                ("Auto-run", "rivet-launcher"),
            ]),
            "lounge.local",
            TEST_REPOSITORY_ID,
        )
        .expect("composes");
        let parsed = rivet_config::Document::parse(&doc).expect("parses");
        assert_eq!(parsed.auto_run(), Some("rivet-launcher"));
        assert!(
            parsed
                .service("rivet-launcher")
                .is_some_and(rivet_config::services::Service::enabled),
            "the application that takes the display must be enabled:\n{doc}"
        );
        assert!(
            parsed
                .service("rivet-displayd")
                .is_some_and(rivet_config::services::Service::enabled),
            "a kiosk must enable the display SERVER the auto-run app draws through, or the \
             app crash-loops on a black screen (dev-01, 2026-09-22):\n{doc}"
        );
        assert_eq!(parsed.effective_control_plane().listen(), "0.0.0.0:7777");

        // A kiosk that has not decided still gets the table, so a later
        // `apply edit` has a key-shaped hole to fill.
        let undecided = compose(
            &answered(&[
                ("Hostname", "lounge"),
                ("Mode", "kiosk"),
                ("Timezone", "UTC"),
            ]),
            "lounge.local",
            TEST_REPOSITORY_ID,
        )
        .expect("composes");
        assert!(undecided.contains("[kiosk]"), "{undecided}");
        assert_eq!(
            rivet_config::Document::parse(&undecided).expect("parses").auto_run(),
            None
        );
    }

    /// **The menu will not compose a document with no zone in it.**
    ///
    /// Not because the schema refuses one — it accepts a document with no
    /// `timezone` and the machine keeps UTC — but because that silence is the
    /// defect: nothing fails, and every timestamp is an hour out.
    #[test]
    fn an_unanswered_timezone_is_refused_before_anything_is_written() {
        let fields = questions();
        let zone = fields
            .iter()
            .find(|f| f.label == "Timezone")
            .expect("the menu asks for a timezone");
        assert!(zone.value.is_empty(), "the timezone must have no default to accept by silence");
        assert!(zones::check(&zone.value).is_err(), "an empty answer must be refused");
        assert!(zones::check("America/Chigago").is_err(), "a typo must be refused");
        assert!(zones::check("America/Chicago").is_ok());
    }

    /// **`ca server rivet-01.local` is the command that produced the defect**,
    /// and it must now compose every name rather than one.
    ///
    /// That exact invocation once minted a leaf with a single SAN and
    /// silently replaced a three-name one on the production fleet server, so
    /// only `rivet-01.local` validated. The suffix a person already typed is
    /// stripped rather than refused or doubled — asking the authority for
    /// `rivet-01.local.local` would be permitted by the constraint and useless
    /// to a browser.
    #[test]
    fn a_hostname_that_already_carries_a_suffix_still_composes_every_name() {
        let suffixes = vec![
            "local".to_owned(),
            "home-fleet.home.arpa".to_owned(),
            "home-fleet.lan".to_owned(),
        ];
        let want = every_name_for("rivet-01", &suffixes);

        for typed in ["rivet-01", "rivet-01.local", "rivet-01.home-fleet.lan", "RIVET-01.Local"] {
            let bare = strip_permitted_suffix(typed, &suffixes);
            assert_eq!(
                every_name_for(&bare, &suffixes).len(),
                3,
                "{typed:?} composed {:?}",
                every_name_for(&bare, &suffixes)
            );
            if typed != "RIVET-01.Local" {
                assert_eq!(every_name_for(&bare, &suffixes), want, "{typed:?}");
            }
        }
    }

    /// **A suffix that is not the authority's is left alone**, and a bare label
    /// that merely looks like one is not eaten.
    #[test]
    fn only_a_permitted_suffix_is_stripped() {
        let suffixes = vec!["local".to_owned(), "home-fleet.lan".to_owned()];
        // Not permitted, so it stays and `issue_from` will refuse it by name.
        assert_eq!(strip_permitted_suffix("rivet-01.example.com", &suffixes), "rivet-01.example.com");
        // The whole name IS the suffix: there is no label left, so nothing is
        // stripped and the caller composes `local.local` rather than `.local`.
        assert_eq!(strip_permitted_suffix("local", &suffixes), "local");
        // A byte-suffix match that is not a label boundary.
        assert_eq!(strip_permitted_suffix("notlocal", &suffixes), "notlocal");
        // The LONGEST permitted suffix wins, so the label is what a person
        // meant rather than `rivet-01.home-fleet`.
        let both = vec!["lan".to_owned(), "home-fleet.lan".to_owned()];
        assert_eq!(strip_permitted_suffix("rivet-01.home-fleet.lan", &both), "rivet-01");
    }

    /// **The menu's composed names are all inside the constraint they came
    /// from**, which is what makes composing from the certificate safe rather
    /// than merely tidy: the authority answers the question, so the answer
    /// cannot be outside the authority.
    #[test]
    fn every_name_the_menu_composes_is_one_the_authority_will_sign() {
        let suffixes = vec!["local".to_owned(), "home-fleet.lan".to_owned()];
        let authority = ca::Authority::root("home-fleet", &suffixes).expect("mints");
        let root = authority.pem().expect("serialises");

        let permitted = ca::permitted_suffixes(&root.certificate).expect("reads back");
        let names = every_name_for("arbor", &permitted);

        ca::issue_from(&root.certificate, &root.key, &names, &names[0])
            .map_err(|e| e.to_string())
            .expect("the authority must sign names composed from its own constraint");
    }

    // -----------------------------------------------------------------------
    // The operator is a certificate that names no host — `.rulings/` R-187 D1.
    // `ca.rs` pins `issue_from` and `Authority::operator`; `stage_tests` pins
    // every role's staged front door. These pin the two workstation commands
    // that mint a machine's leaf, and the list of places anything is minted.
    // -----------------------------------------------------------------------

    /// A scratch authority directory, as `rivet-provision ca` lays one out.
    fn scratch_authority(tag: &str, suffixes: &[&str]) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rivet-provision-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let suffixes: Vec<String> = suffixes.iter().map(|s| (*s).to_owned()).collect();
        let root = ca::Authority::root("test fleet", &suffixes)
            .expect("a root")
            .pem()
            .expect("PEM");
        std::fs::write(dir.join("root.crt"), &root.certificate).expect("written");
        std::fs::write(dir.join("root.key"), &root.key).expect("written");
        dir
    }

    /// What a written certificate file says about its holder.
    fn caller_in(path: &std::path::Path) -> rivetos_tls::Caller {
        let pem = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let (_, parsed) = x509_parser::pem::parse_x509_pem(&pem).expect("a PEM certificate");
        rivetos_tls::classify_caller(&parsed.contents)
    }

    /// **`ca server`, in both of its forms, writes a leaf naming its host**, and
    /// its literal form refuses a list with no name in it rather than minting
    /// one the fleet's front doors would take for the operator.
    #[test]
    fn ca_server_writes_a_leaf_that_names_its_host_in_both_forms() {
        let dir = scratch_authority("ca-server", &["local", "my-fleet.home.arpa"]);
        let at = dir.display().to_string();

        assert_eq!(mint_server(&["arbor".to_owned(), at.clone()]), std::process::ExitCode::SUCCESS);
        assert_eq!(
            caller_in(&dir.join("server-arbor.local.crt")),
            rivetos_tls::Caller::Machine(vec![
                "arbor.local".to_owned(),
                "arbor.my-fleet.home.arpa".to_owned()
            ])
        );

        assert_eq!(
            mint_server(&[NAMES.to_owned(), "vega.local".to_owned(), at.clone()]),
            std::process::ExitCode::SUCCESS
        );
        assert_eq!(
            caller_in(&dir.join("server-vega.local.crt")),
            rivetos_tls::Caller::Machine(vec!["vega.local".to_owned()])
        );

        assert_eq!(
            mint_server(&[NAMES.to_owned(), " , ".to_owned(), at]),
            std::process::ExitCode::FAILURE,
            "a literal list with no name in it was minted"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The menu's mint writes a leaf naming its host**, every permitted name.
    #[test]
    fn the_menus_mint_writes_a_leaf_that_names_its_host() {
        let dir = scratch_authority("menu-mint", &["local", "my-fleet.home.arpa"]);
        let written = mint_for_host_in(&dir, "cedar").expect("minted");
        assert_eq!(
            caller_in(&written),
            rivetos_tls::Caller::Machine(vec![
                "cedar.local".to_owned(),
                "cedar.my-fleet.home.arpa".to_owned()
            ])
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Every place this tool mints a fleet-authority leaf is one the pins above
    /// hold.** A new call to `issue_from`, `.issue(` or `.operator(` outside
    /// `ca.rs` fails here until it is added to this list — and the list is only
    /// honest if a test beside it holds what the new caller writes.
    #[test]
    fn every_place_a_leaf_is_minted_is_one_a_pin_holds() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found: Vec<(String, &str, usize)> = Vec::new();
        for entry in std::fs::read_dir(&src).expect("the source directory") {
            let path = entry.expect("an entry").path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
            if !name.ends_with(".rs") || name.ends_with("_tests.rs") || name == "ca.rs" {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("readable");
            // Only the shipped code: an inline test module is not a mint.
            let shipped = text.split("\nmod tests {").next().unwrap_or(&text);
            for call in ["issue_from(", ".issue(", ".operator("] {
                let count = shipped.matches(call).count();
                if count > 0 {
                    found.push((name.clone(), call, count));
                }
            }
        }
        found.sort();
        assert_eq!(
            found,
            vec![
                // `rivet-provision ca` (`mint_ca`) and the interview's authority.
                ("main.rs".to_owned(), ".operator(", 2),
                // `ca server` (`mint_server`) and the menu (`mint_for_host_in`).
                ("main.rs".to_owned(), "issue_from(", 2),
                // Every fleet role's front door (`stage::compose`).
                ("stage.rs".to_owned(), "issue_from(", 1),
            ],
            "a place that mints a fleet-authority leaf was added or removed; hold what it \
             writes with a test (.rulings/ R-187), then update this list"
        );
    }

    // -- ADR-063 §8: the first-provision guard ------------------------------

    /// An empty scratch directory standing for `~/.rivet/ca` on a workstation
    /// that holds no authority.
    fn scratch_empty(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("rivet-provision-guard-{tag}-{}", std::process::id()))
            .join("ca");
        let _ = std::fs::remove_dir_all(dir.parent().expect("a parent"));
        std::fs::create_dir_all(dir.parent().expect("a parent")).expect("a scratch directory");
        dir
    }

    /// Run the guard with `answers` typed in order, recording every question
    /// read and every mint, in the order they happened. The mint is the real
    /// writer, into `dir`.
    fn run_guard(
        dir: &std::path::Path,
        interactive: bool,
        new_fleet: bool,
        answers: &[&str],
    ) -> (Result<Guarded, String>, Vec<&'static str>) {
        let log = std::cell::RefCell::new(Vec::new());
        let mut typed = answers.iter();
        let mut ask_choice = || {
            log.borrow_mut().push("asked");
            typed.next().map(|a| format!("{a}\n"))
        };
        let mut mint = |d: &std::path::Path| {
            log.borrow_mut().push("minted");
            write_authority(d, "guard-fleet", &["local".to_owned()], "Guard Operator")
        };
        let mut fleet_root = |d: &std::path::Path| {
            log.borrow_mut().push("fleet-root");
            record_fleet_root(d, "dev-fleet", &rivet_core::fleet_root::default_suffixes("dev-fleet"))
        };
        let outcome = first_provision_guard(
            dir,
            interactive,
            new_fleet,
            &mut ask_choice,
            &mut mint,
            &mut fleet_root,
        );
        (outcome, log.into_inner())
    }

    /// **Fleet-root mode mints nothing here** — no root, no key, no browser
    /// certificate — and records the fleet for its server's card; a second run
    /// is not asked again, and the server's request composes from it.
    #[test]
    fn a_fleet_root_fleet_records_its_name_and_mints_nothing_here() {
        let dir = scratch_empty("fleet-root");
        let (outcome, log) = run_guard(&dir, true, false, &["3"]);
        assert_eq!(outcome, Ok(Guarded::FleetRoot));
        assert_eq!(log, vec!["asked", "fleet-root"]);
        for file in ["root.crt", "root.key", "browser.crt", "browser.key"] {
            assert!(!dir.join(file).exists(), "{file} was minted on the workstation");
        }
        let request = fleet_root_request(&dir, "dev-01")
            .expect("reads")
            .expect("recorded");
        assert_eq!(request.fleet, "dev-fleet");
        assert_eq!(request.names(), vec!["dev-01.local", "dev-01.dev-fleet.home.arpa"]);

        let (again, log) = run_guard(&dir, true, false, &["1"]);
        assert_eq!(again, Ok(Guarded::Present), "a fleet-root workstation was asked again");
        assert!(log.is_empty());
        let _ = std::fs::remove_dir_all(dir.parent().expect("a parent"));
    }

    #[test]
    fn a_fleet_root_choice_outside_the_fleets_names_is_refused() {
        let dir = scratch_empty("fleet-root-bad");
        assert!(record_fleet_root(&dir, "dev-fleet", &["com".to_owned()]).is_err());
        assert!(!dir.join(FLEET_ROOT_CHOICE).exists());
        assert_eq!(fleet_root_request(&dir, "dev-01"), Ok(None));
        let _ = std::fs::remove_dir_all(dir.parent().expect("a parent"));
    }

    /// **A workstation that already has an authority is never asked**, and
    /// nothing is minted over it.
    #[test]
    fn a_workstation_with_an_authority_is_never_asked() {
        let dir = scratch_authority("guard-present", &["local"]);
        let before = std::fs::read(dir.join("root.crt")).expect("the root");
        for (interactive, new_fleet) in [(true, false), (false, false), (true, true), (false, true)] {
            let (outcome, log) = run_guard(&dir, interactive, new_fleet, &["1"]);
            assert_eq!(outcome, Ok(Guarded::Present));
            assert!(log.is_empty(), "asked or minted with an authority present: {log:?}");
        }
        assert_eq!(std::fs::read(dir.join("root.crt")).expect("the root"), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The answer comes BEFORE the mint.** The whole guard is this ordering:
    /// a mint that runs first and asks afterwards has already made the second
    /// authority the question exists to prevent. A bare Return is not an
    /// answer, so it is asked again rather than taken as "new".
    #[test]
    fn a_new_fleet_is_minted_only_after_it_is_chosen() {
        let dir = scratch_empty("new");
        let (outcome, log) = run_guard(&dir, true, false, &["", "1"]);
        assert_eq!(outcome, Ok(Guarded::Minted));
        assert_eq!(log, vec!["asked", "asked", "minted"], "the mint must follow the answer");
        for file in ["root.crt", "root.key", "browser.crt", "browser.key", "suffix"] {
            assert!(dir.join(file).is_file(), "{file} was not written");
        }
        let _ = std::fs::remove_dir_all(dir.parent().expect("a parent"));
    }

    /// **Joining mints nothing** — not the root, not a directory for one.
    #[test]
    fn joining_an_existing_fleet_mints_nothing() {
        let dir = scratch_empty("join");
        for answer in ["2", "join", "JOIN"] {
            let (outcome, log) = run_guard(&dir, true, false, &[answer]);
            assert_eq!(outcome, Ok(Guarded::Joining), "{answer:?}");
            assert_eq!(log, vec!["asked"], "{answer:?} minted");
            assert!(!dir.exists(), "{answer:?} created {}", dir.display());
        }
        let (outcome, log) = run_guard(&dir, true, false, &["x"]);
        assert_eq!(outcome, Ok(Guarded::Exited));
        assert_eq!(log, vec!["asked"]);
        assert!(!dir.exists());
        let _ = std::fs::remove_dir_all(dir.parent().expect("a parent"));
    }

    /// **With no terminal, only `--new-fleet` mints.** Without it the run
    /// refuses and reads nothing; with it nothing is asked and the authority
    /// is written.
    #[test]
    fn a_run_with_no_terminal_refuses_unless_told_this_is_a_new_fleet() {
        let dir = scratch_empty("no-terminal");
        let (outcome, log) = run_guard(&dir, false, false, &["1"]);
        assert_eq!(outcome, Ok(Guarded::Refused(NO_ANSWER_REFUSAL.to_owned())));
        assert!(log.is_empty(), "a run with no terminal read or minted: {log:?}");
        assert!(!dir.exists());

        let (outcome, log) = run_guard(&dir, false, true, &[]);
        assert_eq!(outcome, Ok(Guarded::Minted));
        assert_eq!(log, vec!["minted"]);
        assert!(dir.join("root.crt").is_file());
        let _ = std::fs::remove_dir_all(dir.parent().expect("a parent"));
    }

    /// **Input closing mid-question is not an answer**, and mints nothing.
    #[test]
    fn input_closing_before_an_answer_mints_nothing() {
        let dir = scratch_empty("closed");
        let (outcome, log) = run_guard(&dir, true, false, &["maybe"]);
        assert!(matches!(outcome, Ok(Guarded::Refused(_))), "{outcome:?}");
        assert_eq!(log, vec!["asked", "asked"]);
        assert!(!dir.exists());
        let _ = std::fs::remove_dir_all(dir.parent().expect("a parent"));
    }

    /// Every spelling the question accepts, and the ones it refuses.
    #[test]
    fn the_first_fleet_question_reads_numbers_and_words() {
        for (answer, want) in [
            ("1", FirstFleet::New),
            ("new", FirstFleet::New),
            (" New Fleet ", FirstFleet::New),
            ("2", FirstFleet::Join),
            ("join", FirstFleet::Join),
            ("3", FirstFleet::FleetRoot),
            ("fleet-root", FirstFleet::FleetRoot),
            ("x", FirstFleet::Exit),
            ("X", FirstFleet::Exit),
        ] {
            assert_eq!(first_fleet_from_answer(answer), Ok(want), "{answer:?}");
        }
        for refused in ["", "y", "yes", "4", "n"] {
            assert!(first_fleet_from_answer(refused).is_err(), "{refused:?} was accepted");
        }
    }

    // -- I07: publish_bundle never destroys a bundle it fails to replace -----

    /// **A write that cannot even start must leave the old bundle exactly as
    /// it was.** The temporary `publish_bundle` needs is pre-empted as a
    /// directory, so the write it must do before any rename is refused on
    /// every real filesystem — no fake, no permission trick a container's
    /// root user could ignore.
    ///
    /// Fails on the code this replaces: a direct `std::fs::write(path, ..)`
    /// does not look at `<path>.new` at all, so it would overwrite `path`
    /// with the new bytes and return `Ok`, and the assertion below that the
    /// call is refused would panic.
    #[test]
    fn a_refused_write_leaves_the_old_bundle_byte_identical() {
        let scratch = Scratch::new("publish-bundle-refused-write");
        let dest = scratch.0.join("member.staged.txt");
        std::fs::write(&dest, "old bundle bytes").expect("the old bundle is written");
        let temp = scratch.0.join("member.staged.txt.new");
        std::fs::create_dir_all(&temp).expect("a directory standing where the temp file would go");

        publish_bundle(&dest, "new bundle bytes").expect_err("the write is refused");

        let after = std::fs::read_to_string(&dest).expect("the old bundle is still there");
        assert_eq!(after, "old bundle bytes", "a failure before the rename must not touch it");
    }

    /// **A successful publish leaves exactly the new bundle, and absorbs
    /// whatever temporary an earlier interrupted run left behind** rather
    /// than letting it accumulate beside the real file forever.
    ///
    /// Fails on the code this replaces: a direct `std::fs::write` never looks
    /// at `<path>.new`, so the stale temporary seeded below would still be
    /// sitting there afterward and the assertion that it is gone would fail —
    /// which is the same defect this ticket exists to close, just visible as
    /// litter instead of as truncation.
    #[test]
    fn a_successful_publish_leaves_exactly_the_new_bundle_and_absorbs_a_stale_temporary() {
        let scratch = Scratch::new("publish-bundle-absorbs-stale-temp");
        let dest = scratch.0.join("member.staged.txt");
        std::fs::write(&dest, "old bundle bytes").expect("the old bundle is written");
        let temp = scratch.0.join("member.staged.txt.new");
        std::fs::write(&temp, "garbage left by an interrupted run").expect("a stale temporary");

        publish_bundle(&dest, "new bundle bytes").expect("nothing here should refuse");

        let after = std::fs::read_to_string(&dest).expect("the new bundle is there");
        assert_eq!(after, "new bundle bytes", "neither the old bytes nor a mixture");
        assert!(!temp.exists(), "the stale temporary must not survive a successful publish");
    }
}
