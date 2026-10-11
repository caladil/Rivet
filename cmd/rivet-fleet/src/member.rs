//! **Reaching a member** — `ADR-034` §11.4, and the half that acts.
//!
//! The maintainer ruled that deploying an updated document to a member, and
//! trial booting it, belong in this tool (.rulings/ R-330).
//!
//! Everything here goes through `rivet_remote::deploy::Remote`, which is the
//! same port `cargo rivet deploy` performs against — §11.5's ruling, and the
//! reason `rivet-remote` was extracted at all: code that is reachable as a
//! crate is called as a crate, never reached by shelling out to it
//! (.rulings/ R-330).
//!
//! # The payoff `ADR-034` §9 named
//!
//! > **`cargo rivet deploy --device <name>` becomes possible**, resolving
//! > address, access and trust from the record instead of `--host
//! > root@<address>`. That is the concrete payoff and it is small once the
//! > register exists.
//!
//! [`address_of`] is that resolution, and it is the whole of it.
//!
//! # Every sequence here is confirm-first
//!
//! `ADR-012` §7.1 reaches a host tool unchanged: without the word, each of
//! these renders the plan and touches nothing at either end. The plans are
//! deliberately specific about **what is not** done — a document written and
//! not reloaded, a payload installed and not restarted into — because those are
//! the states an operator is most likely to assume away.

use rivet_config::Document;
use rivet_remote::deploy::{DOCUMENT_PATH, Remote};

use crate::register::Device;

/// Where a member is, derived from its record.
///
/// **Derived, never stored.** `AGENTS.md` §5 — a second field holding the
/// address would be a second answer to *where is this machine*, and the one in
/// `[interfaces]` is the one the machine is actually configured with.
///
/// # Errors
///
/// A sentence naming what is missing, in every case where a guess would reach
/// the wrong machine:
///
/// * **no interface carries an address** — the record is `dhcp`, or empty. A
///   DHCP machine's address is not knowable from here **and this is exactly
///   what `rivet-mdnsd` is for** (`PLAN.md` §23): a responder makes
///   `<host>.local` the answer, and until one exists a DHCP member cannot be
///   reached by a tool that only has the register.
/// * **more than one does** — refused rather than guessed, for
///   `Register::resolve`'s reason. Reaching the wrong machine is the one
///   mistake in this tool that is expensive to undo.
pub fn address_of(device: &Device) -> Result<String, String> {
    // **Moved 2026-09-01** to `rivetos_register`, because `cmd/rivet-fleetd`
    // needs the same answer and two implementations of "which address is the
    // machine" would eventually disagree. Kept as a name this crate already
    // spells in a dozen places.
    device.address()
}

/// **Where the register says this machine's control plane listens, against
/// where its own document says** — `ADR-034` §6.1's detection.
///
/// # Why detection is owed, and why it is only detection
///
/// §6.1 rules that the operation changing the listener writes the register and
/// the document **together**, which closes the drift this tool can cause. It
/// does not close the drift a **board-side `apply edit` of `[control-plane]`**
/// causes, and §6.1 says plainly that it cannot be closed there: refusing that
/// edit would need the machine to enforce a fleet-layer rule that `ADR-007`
/// §3.4's layering says it must not.
///
/// > *It can be DETECTED, and that is what the fleet tool owes.*
///
/// # The report names three values, and the third is the point
///
/// The register's, the document's, **and the one this tool actually dialled**.
/// That third is the only honest thing available, because the tool is
/// necessarily talking to the machine on a value that worked — so a reader who
/// saw only the first two would have no way to tell which of them is currently
/// true of the running program.
///
/// This is reachable at all because a `listen` change does not take effect
/// until `rivet-mcpd` restarts: a board whose document was edited to a new
/// address is still answering on the old one, which is exactly when this
/// comparison can be made.
///
/// `None` when they agree, or when the document carries no `[control-plane]`
/// and the record does not either — the two absent states are the same state
/// (see [`crate::register::Device::listen`]).
#[must_use]
pub fn listen_drift(device: &Device, document: &str, dialled: &str) -> Option<String> {
    let theirs = rivet_config::Document::parse(document)
        .ok()?
        .effective_control_plane()
        .listen()
        .to_owned();
    let ours = device
        .listen
        .clone()
        .unwrap_or_else(|| rivet_config::control_plane::DEFAULT_LISTEN.to_owned());
    if ours == theirs {
        return None;
    }
    Some(format!(
        "DRIFT - the register and the machine disagree about where this control plane listens.\n\
         \x20 register  {ours}\n\
         \x20 machine   {theirs}  (its own document says so)\n\
         \x20 reached   {dialled}  - this is the value that WORKED, and it is the only one \
         either of the two above can be checked against.\n\
         \x20 why       a listen change takes effect when rivet-mcpd restarts, so a board \
         whose document was edited is still answering at the old address. ADR-034 6.1 rules \
         that changing the listener is a FLEET operation writing both - `set listen` - and \
         that a board-side `apply edit` of [control-plane] cannot be refused at the machine.\n\
         \x20 fix       `set listen --device {name} --listen {theirs} --confirm` records what \
         the machine already says, or --listen {ours} to put the machine back.\n",
        name = device.name,
    ))
}

/// **A transport that can both carry a document and run a catalogue
/// operation** — the two halves this module needs from one connection.
///
/// # Why this exists, and why it is not a third trait's worth of design
///
/// `rivet_remote::deploy::Remote` carries the document methods.
/// `rivet_remote::tls::CallsTools` carries a tool call. `Tls` implements both
/// and nothing here needs to know that, but a `&mut dyn Remote` cannot make a
/// tool call and a `&dyn CallsTools` cannot put a document — so the functions
/// below would each have to take two references to the same connection.
///
/// **[`Member::perform`] is the one thing this adds**, and it adds no
/// behaviour: it is `rivet_remote::command::perform`, which binds a word list
/// to `rivet_core::operation::CATALOGUE` and sends it as a tool call.
///
/// # `Remote::ask` is what this replaces, and the difference is the point
///
/// `ask` takes a word list because the transports it was written for handed the
/// machine a shell, and `Tls`'s implementation of it refuses in exactly those
/// words: *"MCP is the CATALOGUE, not a command line."* That refusal is correct
/// and is left standing. What changed is that `rivet_remote::command` now
/// exists — *"the translation that refusal declines to guess at"* — so this
/// module names an operation and lets the catalogue compose the call, rather
/// than handing words to something that would have handed them to a shell.
///
/// # One implementation, written out rather than blanket
///
/// A blanket `impl<T: Remote + CallsTools>` would have been shorter and is
/// wrong here twice over: coherence cannot admit it beside [`Nothing`], which
/// is `Remote` and deliberately not `CallsTools`; and there is exactly **one**
/// real transport now, so a blanket impl would be generality over a set of one.
/// `ADR-034` §11.5.1 is why that set has one member and is not going to grow.
pub trait Member: Remote {
    /// Run one catalogue operation on the machine and give back its answer.
    ///
    /// # Errors
    ///
    /// This end's refusal when the words are not a catalogue operation, and
    /// otherwise the machine's own words.
    fn perform(&self, words: &[&str]) -> Result<String, String>;
}

impl Member for rivet_remote::tls::Tls {
    fn perform(&self, words: &[&str]) -> Result<String, String> {
        rivet_remote::command::perform(self, words)
    }
}

/// **A transport for a plan, which reaches nothing and is asked nothing.**
///
/// Every sequence in this module is confirm-first and touches no machine
/// without the word, so a plan needs a `Member`-shaped value and needs it to
/// stay unused. Passing one of these says that in the type rather than in a
/// comment: **each method is `unreachable!`**, so a plan path that grew a call
/// to a machine would fail in a test rather than open a connection.
///
/// It also means a plan needs no `--identity`. Composing a document and showing
/// it is a local act, and requiring an operator identity to see one would be
/// this tool asking for a key in order to print.
pub struct Nothing;

impl Remote for Nothing {
    fn push(&mut self, _local: &std::path::Path, _remote: &str) -> Result<String, String> {
        unreachable!("a plan reaches no machine")
    }
    fn blake3(&mut self, _remote: &str) -> Result<[u8; 32], String> {
        unreachable!("a plan reaches no machine")
    }
    fn install(&mut self, _remote: &str) -> Result<String, String> {
        unreachable!("a plan reaches no machine")
    }
    fn read_document(&mut self) -> Result<Option<String>, String> {
        unreachable!("a plan reaches no machine")
    }
    fn edit_document(
        &mut self,
        _change: &rivet_remote::deploy::ConfigChange,
    ) -> Result<String, String> {
        unreachable!("a plan reaches no machine")
    }
    fn put_document(&mut self, _text: &str) -> Result<(), String> {
        unreachable!("a plan reaches no machine")
    }
    fn ask(&mut self, _words: &[&str]) -> Result<String, String> {
        unreachable!("a plan reaches no machine")
    }
}

impl Member for Nothing {
    fn perform(&self, _words: &[&str]) -> Result<String, String> {
        unreachable!("a plan reaches no machine")
    }
}

/// **What the machine holds, against what the register would write** —
/// `ADR-034` §6.2's comparison, in the one place both documents exist.
///
/// The machine's text goes through **its own parser** when that parser will
/// take it, so the comparison is over what the document *means* rather than
/// over how somebody typed it: a hand-edited document and a canonical one
/// differ in whitespace, table order and string style, none of which is a
/// value. When it does not parse, the raw text is compared instead and the
/// caller is handed a sentence saying so — refusing to look would be worse
/// than looking at the text, and a silent downgrade would be worse than both.
///
/// # Errors
///
/// Whatever `compose` refused, or `toml_edit`'s words for text that is not
/// TOML at all.
pub fn compare(
    device: &Device,
    fleet: &crate::register::Fleet,
    machine: &str,
) -> Result<(crate::diff::Diff, Option<String>), String> {
    let composed =
        crate::compose::provisioning_toml(device, fleet).map_err(|e| e.to_string())?;
    let (text, note) = match Document::parse(machine) {
        Ok(document) => (document.to_toml(), None),
        Err(e) => (
            machine.to_owned(),
            Some(format!(
                "note     this machine's own document does not parse here - {e}. It is \
                 compared as TEXT rather than as a document, so what follows is over its \
                 keys and not over their meaning.\n"
            )),
        ),
    };
    let theirs = crate::diff::leaves(&text)?;
    let ours = crate::diff::leaves(&composed)?;
    Ok((crate::diff::between(&theirs, &ours), note))
}

/// **`ADR-034` §6.2: refuse to erase what the register does not know.**
///
/// A removal refuses; a change is reported and written. §6.2.1 is where that
/// asymmetry is argued — the short form is that an absence in the register is
/// never a request, because there is no way to spell one, while a differing
/// value is the operator's intent nearly always.
///
/// `seen` is `None` only for a machine that ANSWERED that it holds no
/// document. A read that failed is not that answer, and [`put_document`]
/// refuses before this is reached.
///
/// # Errors
///
/// The refusal naming each value the write would remove, or whatever
/// [`compare`] refused.
pub fn guard(
    device: &Device,
    fleet: &crate::register::Fleet,
    seen: Option<&str>,
    report: &mut String,
) -> Result<(), String> {
    let Some(machine) = seen else {
        // **A machine with no document has nothing to lose**, and this is the
        // first apply onto a fresh board. §6.2 must not make that harder; it
        // exists for the board that already holds something.
        report.push_str(
            "checked  this machine holds no document yet, so nothing can be removed by \
             writing one (ADR-034 6.2)\n",
        );
        return Ok(());
    };
    let (found, note) = compare(device, fleet, machine)?;
    if let Some(said) = note {
        report.push_str(&said);
    }
    if let Some(refusal) = crate::diff::refusal(device, &found) {
        return Err(refusal);
    }
    report.push_str(
        "checked  nothing this machine holds would be REMOVED by this write (ADR-034 6.2)\n",
    );
    if let Some(said) = crate::diff::changes(&found) {
        report.push_str(&said);
    }
    Ok(())
}

/// What `apply document` did, or would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentOutcome {
    /// What the machine said the reload did.
    pub reloaded: String,
    /// Whether the bytes read back are the bytes sent.
    pub confirmed_bytes: bool,
}

/// Compose this device's document, put it on the machine, read it back, and
/// make the machine re-read it.
///
/// # The order, and why the read-back is between them
///
/// 1. compose — through `rivet-config`'s own parser, so a document the machine
///    would refuse is refused here (`crate::compose`);
/// 2. write it to `/state/config/machine.toml`;
/// 3. **read it back and compare**, for `cargo rivet deploy`'s reason exactly:
///    without it a truncated write surfaces later as a *parse* refusal, and an
///    operator reading *"unknown key"* goes and looks at their document;
/// 4. only then `rivet apply reload confirm`.
///
/// **Step 4 is skipped when step 3 fails**, and the report says the machine is
/// running its old document — which is true, and is the safe half: a supervisor
/// that reloaded a half-written file is the one outcome worth avoiding here.
///
/// # Errors
///
/// Whatever the composition, the transfer or the machine refused, verbatim.
pub fn put_document(
    device: &Device,
    fleet: &crate::register::Fleet,
    remote: &mut dyn Member,
    confirmed: bool,
    report: &mut String,
) -> Result<Option<DocumentOutcome>, String> {
    // **The strict composer.** A write to a live machine is a document
    // leaving the register for good, exactly like an export or a card build —
    // `crate::compose`'s own header names the three together - so this is
    // never allowed to send `SPEC-05` §5 an incomplete document, plan or not.
    let composed: Document =
        crate::compose::provisioning_complete(device, fleet).map_err(|e| e.to_string())?;
    let text = composed.to_toml();

    if !confirmed {
        report.push_str(&format!(
            "PLAN - nothing has been composed onto the machine.\n\
             \n\
             would write   {DOCUMENT_PATH} on {}\n\
             would compare the machine's own document against this one FIRST, and refuse if \
             this write would remove anything the register does not hold (ADR-034 6.2). \
             That check needs the machine, and a plan reaches none - so it happens when you \
             add --confirm, not here.\n\
             would check   the bytes read back are the bytes sent\n\
             would run     rivet apply reload confirm\n\
             \n\
             It would NOT restart the machine. A reload re-reads the document and applies what \
             a running machine may take (ADR-032 3.2); everything else waits for the next boot.\n\
             \n\
             The document that would be written:\n{}\n\
             Add `--confirm` to do it.\n",
            device.name,
            indent(&text),
        ));
        return Ok(None);
    }

    // **`ADR-034` §6's removal guard, at the one boundary every full-document write
    // crosses** — `apply document`, `set listen` and the TUI's apply alike.
    // The machine's document is read here, at the point of use, and a read
    // that FAILED refuses the write: it is not a machine with nothing to lose,
    // it is a machine nobody has looked at. The refusal carries the report so
    // far, because a caller that prints only the error would drop it.
    let held = remote.read_document().map_err(|e| {
        format!(
            "{report}REFUSED this machine's document at {DOCUMENT_PATH} could not be read, so \
             nothing can say what a full write would remove, and NOTHING HAS BEEN WRITTEN \
             (ADR-034 6.2). Detail: {e}"
        )
    })?;
    if let Err(refusal) = guard(device, fleet, held.as_deref(), report) {
        return Err(format!("{report}{refusal}"));
    }

    remote.put_document(&text)?;
    report.push_str(&format!("wrote    {DOCUMENT_PATH} ({} bytes)\n", text.len()));

    let seen = remote
        .read_document()?
        .ok_or("the document was written and reads back as absent, so the machine is running \
                whatever it was running before. Nothing was reloaded.")?;
    if seen != text {
        return Err(format!(
            "the document on the machine is NOT the document that was sent, so nothing was \
             reloaded and the machine is still running its old one.\n\
             \x20 sent      {} bytes\n\
             \x20 read back {} bytes\n\
             This is a fault in the transfer rather than in the document - it was composed \
             through the machine's own parser before it was sent. Send it again.",
            text.len(),
            seen.len()
        ));
    }
    report.push_str("checked  the machine's copy is byte-identical\n");

    let reloaded = remote.perform(&["apply", "reload", "confirm"])?;
    report.push_str("reloaded requested\n");
    if !reloaded.trim().is_empty() {
        report.push_str(&indent(reloaded.trim()));
    }
    report.push_str(
        "\nA reload applies what a running machine may take - `services.*.enabled`, except for \
         the two whose loss takes away the means of undoing it (ADR-032 3.2.1). Everything else \
         in that document takes effect at the next boot.\n",
    );

    Ok(Some(DocumentOutcome {
        reloaded,
        confirmed_bytes: true,
    }))
}

/// Ask a member what it is doing — `rivet get status`, `ADR-032` §3.3.
///
/// **The machine's own answer, quoted rather than reformatted.** A fleet tool
/// that rendered a machine's status in its own words would be a second
/// vocabulary for one fact, and an operator comparing what the tool said with
/// what `rivet get status` says at a prompt would have two things to reconcile.
///
/// # Errors
///
/// Whatever the machine or the connection said.
pub fn status(remote: &mut dyn Member) -> Result<String, String> {
    remote.perform(&["get", "status"])
}

/// Arm a trial boot of the other slot and restart into it — `ADR-018` §5.
///
/// **The one operation here that takes a machine away**, and the plan says so
/// in the words that matter: if the new slot comes up unreachable, nothing in
/// this tool can recover it. `ADR-018` §5.1's rollback needs the *next* reset,
/// and on a headless board somebody has to cause one.
///
/// # Errors
///
/// Whatever the machine said.
pub fn trial_restart(
    device: &Device,
    remote: &mut dyn Member,
    confirmed: bool,
    report: &mut String,
) -> Result<Option<String>, String> {
    if !confirmed {
        report.push_str(&format!(
            "PLAN - {} has not been touched.\n\
             \n\
             would run     rivet apply trial-restart confirm\n\
             \n\
             THIS TAKES THE MACHINE DOWN. It restarts once into the other A/B slot; if that \
             slot boots and confirms itself, it becomes the machine. If it boots and cannot be \
             REACHED, nothing here can recover it - ADR-018 5.1's rollback happens at the next \
             reset, and on a headless board somebody has to cause one.\n\
             \n\
             Add `--confirm` to do it.\n",
            device.name,
        ));
        return Ok(None);
    }
    // The connection dies with the machine, so a non-zero exit and a dropped
    // connection are both ordinary here — which is why the answer is reported
    // rather than checked.
    let said = remote
        .perform(&["apply", "trial-restart", "confirm"])
        .unwrap_or_else(|e| format!("the connection ended: {e}"));
    report.push_str(&format!(
        "requested a trial restart of {}\n{}\n\
         The connection ends with the machine. Nothing here can tell you whether the other slot \
         came up - ask it: rivet-fleet get status --device {}\n",
        device.name,
        indent(said.trim()),
        device.name,
    ));
    Ok(Some(said))
}

fn indent(text: &str) -> String {
    text.lines().map(|line| format!("  {line}\n")).collect()
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::register::{Fleet, Interface};
    use rivet_remote::deploy::ConfigChange;
    use std::path::Path;

    fn device() -> Device {
        let mut device = Device {
            id: "0011223344556677".to_owned(),
            name: "kitchen-pi".to_owned(),
            // None of this module's tests are about SPEC-05 5's completeness
            // gates - `put_document` composes through `provisioning_complete`
            // because a live write is a document leaving the register for
            // good, so the fixture answers what that gate asks for.
            timezone: Some("UTC".to_owned()),
            ..Device::default()
        };
        device.interfaces.insert(
            "eth0".to_owned(),
            Interface {
                method: "static".to_owned(),
                address: Some("192.168.1.50/24".to_owned()),
                gateway: Some("192.168.1.1".to_owned()),
            },
        );
        device
    }

    /// A machine that takes whatever it is given and hands back what it holds.
    #[derive(Default)]
    struct FakeMachine {
        held: Option<String>,
        /// What `read_document` answers instead, when the transfer is being
        /// made to go wrong.
        instead: Option<String>,
        /// **Interior mutability, because `Member::perform` takes `&self`.**
        /// It does so because `rivet_remote::tls::CallsTools` does, and that
        /// trait takes `&self` because a tool call does not change the
        /// transport — a fake that needed `&mut` would be inventing a
        /// requirement the real one does not have.
        asked: std::cell::RefCell<Vec<Vec<String>>>,
        refuse_ask: bool,
    }

    impl FakeMachine {
        /// Every operation this machine was asked to perform, in order.
        fn asked(&self) -> Vec<Vec<String>> {
            self.asked.borrow().clone()
        }
    }

    impl Remote for FakeMachine {
        fn push(&mut self, _local: &Path, _remote_path: &str) -> Result<String, String> {
            unreachable!("apply document does not push files")
        }
        fn blake3(&mut self, _remote_path: &str) -> Result<[u8; 32], String> {
            unreachable!("apply document does not hash")
        }
        fn install(&mut self, _remote_path: &str) -> Result<String, String> {
            unreachable!("apply document does not install")
        }
        fn read_document(&mut self) -> Result<Option<String>, String> {
            Ok(self.instead.clone().or_else(|| self.held.clone()))
        }
        fn edit_document(&mut self, _change: &ConfigChange) -> Result<String, String> {
            unreachable!("apply document replaces rather than edits")
        }
        fn put_document(&mut self, text: &str) -> Result<(), String> {
            self.held = Some(text.to_owned());
            Ok(())
        }
        fn ask(&mut self, _words: &[&str]) -> Result<String, String> {
            unreachable!("nothing in this crate speaks a word list at a transport any more")
        }
    }

    /// **The fake records the OPERATION, not the tool call.** A real `Member`
    /// binds the words through the catalogue before they reach a socket; what
    /// these tests are about is which operation was asked for, so the fake
    /// implements `Member` directly and the binding is `rivet-remote`'s own
    /// business, tested there.
    impl Member for FakeMachine {
        fn perform(&self, words: &[&str]) -> Result<String, String> {
            // Checked against the catalogue even here, so a test cannot assert
            // that this tool asks for something the machine has no tool for.
            rivet_remote::command::translate(words)?;
            #[allow(clippy::unwrap_used)]
            self.asked
                .borrow_mut()
                .push(words.iter().map(|w| (*w).to_string()).collect());
            if self.refuse_ask {
                return Err("the machine said no".to_owned());
            }
            Ok(String::new())
        }
    }

    // ---- resolving where a member is ---------------------------------------

    #[test]
    fn a_members_address_comes_out_of_its_record() {
        assert_eq!(address_of(&device()).expect("one address"), "192.168.1.50");
    }

    /// **A DHCP member cannot be reached from the register, and the refusal
    /// names the thing that would fix it.** That is not a placeholder: `PLAN.md`
    /// §23's `rivet-mdnsd` is precisely the answer, and an operator who meets
    /// this should learn that rather than conclude the tool is broken.
    #[test]
    fn a_member_with_no_address_is_refused_and_the_refusal_names_mdns() {
        let mut dhcp = device();
        dhcp.interfaces.insert(
            "eth0".to_owned(),
            Interface {
                method: "dhcp".to_owned(),
                address: None,
                gateway: None,
            },
        );
        let error = address_of(&dhcp).expect_err("no address");
        assert!(error.contains("rivet-mdnsd"), "{error}");
        assert!(error.contains("Nothing has been sent"), "{error}");
    }

    /// Two addresses is a refusal, for `Register::resolve`'s reason: reaching
    /// the wrong machine is the expensive mistake here.
    #[test]
    fn a_member_with_two_addresses_is_refused_rather_than_guessed_at() {
        let mut two = device();
        two.interfaces.insert(
            "wlan0".to_owned(),
            Interface {
                method: "static".to_owned(),
                address: Some("10.0.0.9/24".to_owned()),
                gateway: None,
            },
        );
        let error = address_of(&two).expect_err("two addresses");
        assert!(error.contains("192.168.1.50"), "{error}");
        assert!(error.contains("10.0.0.9"), "{error}");
    }

    // ---- apply document ----------------------------------------------------

    /// **Without the word, neither end is touched** — `ADR-012` §7.1 in a host
    /// tool, and the plan shows the document rather than describing it.
    #[test]
    fn without_the_word_nothing_is_written_and_nothing_is_reloaded() {
        let mut machine = FakeMachine::default();
        let mut report = String::new();
        let outcome = put_document(&device(), &Fleet::default(), &mut machine, false, &mut report)
            .expect("a plan");
        assert_eq!(outcome, None);
        assert_eq!(machine.held, None);
        assert!(machine.asked().is_empty());
        assert!(report.contains("PLAN"), "{report}");
        assert!(report.contains("would run     rivet apply reload confirm"), "{report}");
        // The composed document is in the plan, so the operator sees what would
        // land before it lands.
        assert!(report.contains("[network.eth0]"), "{report}");
    }

    /// The order is the contract: written, read back, and only then reloaded.
    #[test]
    fn the_document_is_read_back_before_the_machine_is_asked_to_reload() {
        let mut machine = FakeMachine::default();
        let mut report = String::new();
        let outcome = put_document(&device(), &Fleet::default(), &mut machine, true, &mut report)
            .expect("it worked")
            .expect("an outcome");
        assert!(outcome.confirmed_bytes);
        assert_eq!(machine.asked(), vec![vec!["apply", "reload", "confirm"]]);
        assert!(report.contains("byte-identical"), "{report}");

        // And what landed is what `rivet-config` renders, not this tool's own
        // text — so a machine reading it back gets a canonical document.
        let held = machine.held.expect("something was written");
        assert_eq!(Document::parse(&held).expect("valid").to_toml(), held);
    }

    /// **The branch that matters.** A transfer that arrived wrong must not be
    /// reloaded: a supervisor re-reading a half-written document is the one
    /// outcome here worth avoiding.
    #[test]
    fn a_document_that_reads_back_differently_is_not_reloaded() {
        let mut machine = FakeMachine {
            instead: Some("version = 1\n".to_owned()),
            ..FakeMachine::default()
        };
        let mut report = String::new();
        let error = put_document(&device(), &Fleet::default(), &mut machine, true, &mut report)
            .expect_err("the bytes differ");
        assert!(error.contains("NOT the document that was sent"), "{error}");
        assert!(error.contains("still running its old one"), "{error}");
        assert!(machine.asked().is_empty(), "it reloaded anyway: {:?}", machine.asked());
    }

    /// A machine that has no document after it was written to is not reloaded
    /// either, and the refusal says what state that leaves.
    #[test]
    fn a_document_that_reads_back_absent_is_not_reloaded() {
        struct Forgetful;
        impl Remote for Forgetful {
            fn push(&mut self, _l: &Path, _r: &str) -> Result<String, String> {
                unreachable!()
            }
            fn blake3(&mut self, _r: &str) -> Result<[u8; 32], String> {
                unreachable!()
            }
            fn install(&mut self, _r: &str) -> Result<String, String> {
                unreachable!()
            }
            fn read_document(&mut self) -> Result<Option<String>, String> {
                Ok(None)
            }
            fn edit_document(&mut self, _c: &ConfigChange) -> Result<String, String> {
                unreachable!()
            }
            fn put_document(&mut self, _text: &str) -> Result<(), String> {
                Ok(())
            }
            fn ask(&mut self, _words: &[&str]) -> Result<String, String> {
                unreachable!("nothing speaks a word list at a transport any more")
            }
        }
        impl Member for Forgetful {
            fn perform(&self, _words: &[&str]) -> Result<String, String> {
                panic!("it must not reload a document it cannot read back")
            }
        }
        let mut report = String::new();
        let error = put_document(
            &device(),
            &Fleet::default(),
            &mut Forgetful,
            true,
            &mut report,
        )
        .expect_err("absent");
        assert!(error.contains("Nothing was reloaded"), "{error}");
    }

    /// **Every full-document write refuses what `apply document` refuses** —
    /// the external audit's F48. The TUI's apply calls this function and
    /// nothing else, so the removal guard has to live here for it to apply
    /// there: a machine holding a listener and a service the register does
    /// not know keeps both, and nothing is written.
    #[test]
    fn a_full_document_write_refuses_to_remove_what_the_register_does_not_hold() {
        let mut running = device();
        running.listen = Some("0.0.0.0:7777".to_owned());
        running.services.insert("rivet-mcpd".to_owned(), true);
        let held = crate::compose::provisioning_toml(&running, &Fleet::default())
            .expect("composable");
        let mut machine = FakeMachine {
            held: Some(held.clone()),
            ..FakeMachine::default()
        };
        let mut report = String::new();
        let refusal = put_document(&device(), &Fleet::default(), &mut machine, true, &mut report)
            .expect_err("both values would be removed");
        assert!(refusal.contains("control-plane.listen"), "{refusal}");
        assert!(refusal.contains("NOTHING HAS BEEN WRITTEN"), "{refusal}");
        assert_eq!(machine.held.as_deref(), Some(held.as_str()), "the document was replaced");
        assert!(machine.asked().is_empty(), "it reloaded: {:?}", machine.asked());
    }

    /// **And a document that cannot be read is not one with nothing in it.**
    /// The write is refused rather than treated as a first apply onto a
    /// fresh board.
    #[test]
    fn a_full_document_write_refuses_when_the_machines_document_cannot_be_read() {
        struct Unreadable {
            written: bool,
        }
        impl Remote for Unreadable {
            fn push(&mut self, _l: &Path, _r: &str) -> Result<String, String> {
                unreachable!()
            }
            fn blake3(&mut self, _r: &str) -> Result<[u8; 32], String> {
                unreachable!()
            }
            fn install(&mut self, _r: &str) -> Result<String, String> {
                unreachable!()
            }
            fn read_document(&mut self) -> Result<Option<String>, String> {
                Err("the connection dropped".to_owned())
            }
            fn edit_document(&mut self, _c: &ConfigChange) -> Result<String, String> {
                unreachable!()
            }
            fn put_document(&mut self, _text: &str) -> Result<(), String> {
                self.written = true;
                Ok(())
            }
            fn ask(&mut self, _words: &[&str]) -> Result<String, String> {
                unreachable!("nothing speaks a word list at a transport any more")
            }
        }
        impl Member for Unreadable {
            fn perform(&self, _words: &[&str]) -> Result<String, String> {
                panic!("nothing may be reloaded after a refused write")
            }
        }
        let mut machine = Unreadable { written: false };
        let mut report = String::new();
        put_document(&device(), &Fleet::default(), &mut machine, true, &mut report)
            .expect_err("an unread document must not be overwritten");
        assert!(!machine.written, "the document was replaced without being read");
    }

    /// A record that composes nothing reachable is refused **before** the
    /// machine is opened, so a bad record costs no connection.
    #[test]
    fn a_record_that_will_not_compose_is_refused_before_anything_is_sent() {
        let mut stranded = device();
        stranded.interfaces.clear();
        let mut machine = FakeMachine::default();
        let mut report = String::new();
        let error = put_document(&stranded, &Fleet::default(), &mut machine, true, &mut report)
            .expect_err("no interface");
        assert!(error.contains("network interface"), "{error}");
        assert_eq!(machine.held, None);
    }

    // ---- the machine's own words -------------------------------------------

    /// The status is asked in the machine's vocabulary and returned in the
    /// machine's words.
    #[test]
    fn status_asks_the_machine_and_quotes_it() {
        let mut machine = FakeMachine::default();
        status(&mut machine).expect("it answered");
        assert_eq!(machine.asked(), vec![vec!["get", "status"]]);
    }

    // ---- the one that takes a machine away ---------------------------------

    #[test]
    fn a_trial_restart_says_what_it_cannot_recover_from_before_it_is_confirmed() {
        let mut machine = FakeMachine::default();
        let mut report = String::new();
        assert_eq!(
            trial_restart(&device(), &mut machine, false, &mut report).expect("a plan"),
            None
        );
        assert!(machine.asked().is_empty());
        assert!(report.contains("TAKES THE MACHINE DOWN"), "{report}");
        assert!(report.contains("nothing here can recover it"), "{report}");
    }

    /// **A connection that dies is the expected outcome**, not a failure: the
    /// machine is going down and the session goes with it.
    #[test]
    fn a_trial_restart_treats_a_dropped_connection_as_ordinary() {
        let mut machine = FakeMachine {
            refuse_ask: true,
            ..FakeMachine::default()
        };
        let mut report = String::new();
        trial_restart(&device(), &mut machine, true, &mut report)
            .expect("a dropped connection is not an error here");
        assert_eq!(machine.asked(), vec![vec!["apply", "trial-restart", "confirm"]]);
        assert!(report.contains("connection ends with the machine"), "{report}");
    }
}
