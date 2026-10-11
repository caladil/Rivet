//! **Giving this machine a name** — `rivet_core::hostname`, ruled 2026-08-26.
//!
//! One name, three readers: `sethostname(2)`, `/etc/hosts`, and — when it
//! exists — `cmd/rivet-mdnsd`'s `<hostname>.local`. `ADR-011`: one authority,
//! and the failure the ruling refused was `uname` saying one thing while the
//! network answered to another.
//!
//! # Why this is the supervisor's and not PID 1's
//!
//! `ADR-017` §3.1.5's list is deliberately short *"because everything on this
//! list runs in a process whose failure panics the kernel"*, and §3.1.8 already
//! settled the neighbouring case: the `/etc` projection is the supervisor's.
//! The hostname belongs beside it and for the same reasons —
//!
//! * **It is a fact from the document**, and the supervisor is what reads the
//!   document for everything else that is not a mount.
//! * **`/etc/hosts` carries it**, so the two must be decided together or a
//!   machine could `uname` one name and resolve another. Splitting them across
//!   two processes is the two-authority failure with extra steps.
//! * **A failure here must not stop a boot.** A machine with no name still
//!   boots, still takes an address, and is still reachable — which is exactly
//!   `ADR-017` §6.2's bootstrap test coming out on the *useful* side. In PID 1
//!   the same failure would be one panic away from a kernel that cannot start.
//!
//! # The derivation, and the one thing it must not do
//!
//! An absent `hostname` key means *the operator said nothing*, so the machine
//! names itself. The derivation must be **stable** and **distinct**, and the
//! second matters more: two machines answering to one `.local` name is a
//! network that has learned a lie, which is `PLAN.md` §23.2's fourth bullet
//! arriving before the service that would cause it.
//!
//! So the input is [`SERIAL_SOURCE`] — a per-machine fact — and a machine that
//! cannot read one **keeps `(none)` and says so at `Warn`**. That is
//! `AGENTS.md` §5 applied to a name: refuse rather than guess, because the
//! guess here is not merely wrong, it collides.

use rivet_core::hostname::{self, Hostname};

use crate::journal::{Journal, Severity, Value};
use crate::kernel::Kernel;

/// Where a per-machine fact is read from.
///
/// `/proc/cpuinfo`'s `Serial` line. The test fixture below is that file as a
/// Raspberry Pi 4 prints it, **read off a board rather than out of
/// documentation** (`AGENTS.md` §5), with the board's own serial replaced: a
/// hardware serial is a fact about one machine, and this module exists so that
/// it never leaves it.
pub const SERIAL_SOURCE: &str = "/proc/cpuinfo";

/// The field in it that distinguishes one board from another.
pub const SERIAL_FIELD: &str = "Serial";

/// What this machine ended up called, and where the name came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Naming {
    /// The document said so.
    Stated(Hostname),
    /// The document said nothing and the machine named itself.
    Derived(Hostname),
    /// The document said nothing and the machine could not distinguish itself,
    /// so it has no name. See the module header for why that is a refusal
    /// rather than a fallback.
    Nameless,
}

impl Naming {
    /// The name, if there is one.
    #[must_use]
    pub const fn hostname(&self) -> Option<&Hostname> {
        match self {
            Self::Stated(name) | Self::Derived(name) => Some(name),
            Self::Nameless => None,
        }
    }
}

/// Pull the machine's serial out of `/proc/cpuinfo`'s text.
///
/// **A line scan rather than a parser**, for `parse_measure_blake3`'s reason:
/// this workspace has no INI reader and taking one to read one field would be a
/// dependency decision. The format is `Serial<tabs>: <value>` and it has been
/// that on every Raspberry Pi kernel this project has met.
#[must_use]
pub fn serial_in(cpuinfo: &str) -> Option<&str> {
    cpuinfo.lines().find_map(|line| {
        let (field, value) = line.split_once(':')?;
        (field.trim() == SERIAL_FIELD).then(|| value.trim())
    })
}

/// Decide what this machine is called, and tell the kernel.
///
/// Returns the outcome rather than a `Result`: **every branch here is
/// survivable**, and a boot that stopped because a name could not be set would
/// be trading a machine that works for a machine that is correctly labelled.
/// Each failure is journalled at the severity it deserves and the boot goes on.
pub fn name_this_machine(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    document: &rivet_config::Document,
) -> Naming {
    let naming = decide(k, journal, document);

    if let Some(name) = naming.hostname() {
        match k.set_hostname(name.as_str()) {
            Ok(()) => note(
                k,
                journal,
                Severity::Info,
                "this machine has a name",
                vec![
                    ("hostname".to_string(), Value::Str(name.to_string())),
                    (
                        "from".to_string(),
                        Value::Str(
                            match naming {
                                Naming::Stated(_) => "the document",
                                _ => "this machine's own serial, because the document named none",
                            }
                            .to_string(),
                        ),
                    ),
                ],
            ),
            // **Survivable, and worth saying loudly.** `sethostname(2)` wants
            // `CAP_SYS_ADMIN`, so `EPERM` is what an unprivileged supervisor
            // gets — and `/etc/hosts` will still carry the name, so the machine
            // resolves it even though `uname` does not report it. That is two
            // answers to one question, which is exactly what this ruling was
            // meant to prevent, so it is `Warn` rather than `Info`.
            Err(errno) => note(
                k,
                journal,
                Severity::Warn,
                "the kernel would not take this machine's name, so `uname -n` and /etc/hosts now \
                 disagree — the name is resolvable and not reported",
                vec![
                    ("hostname".to_string(), Value::Str(name.to_string())),
                    ("errno".to_string(), Value::Str(errno.to_string())),
                ],
            ),
        }
    }

    naming
}

/// Which name applies, without telling anybody.
///
/// Split out so the decision is testable with no syscall — the same shape
/// `crate::etc`'s rendering takes.
fn decide(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    document: &rivet_config::Document,
) -> Naming {
    if let Some(stated) = document.hostname() {
        return Naming::Stated(stated.clone());
    }

    let cpuinfo = match k.read_file(SERIAL_SOURCE) {
        Ok(text) => text,
        Err(errno) => {
            note(
                k,
                journal,
                Severity::Warn,
                "this machine has no name: the document names none and its serial could not be \
                 read, so there is nothing to derive one from. A derived name has to be DISTINCT \
                 or two machines answer to one .local name, so nothing is guessed",
                vec![
                    ("source".to_string(), Value::Str(SERIAL_SOURCE.to_string())),
                    ("errno".to_string(), Value::Str(errno.to_string())),
                ],
            );
            return Naming::Nameless;
        }
    };

    let serial = serial_in(&cpuinfo).unwrap_or("");
    if !hostname::distinguishes_a_machine(serial) {
        note(
            k,
            journal,
            Severity::Warn,
            "this machine has no name: the document names none and this platform reports no \
             serial to derive one from. Set `hostname` in the document to give it one",
            vec![
                ("source".to_string(), Value::Str(SERIAL_SOURCE.to_string())),
                ("field".to_string(), Value::Str(SERIAL_FIELD.to_string())),
            ],
        );
        return Naming::Nameless;
    }

    // Hashed rather than used directly: the name reaches a multicast group
    // every device on the link can hear, and a hardware serial announced to the
    // LAN is a fact about this machine that nothing needed.
    // `rivet_core::hostname::derived` carries the argument.
    let digest = {
        use rivet_core::package::port::Digest as _;
        let mut hash = rivet_crypto::Blake3::new();
        hash.start();
        hash.write(serial.as_bytes());
        hash.finish()
    };

    hostname::derived(&digest).map_or(Naming::Nameless, Naming::Derived)
}

fn note(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    severity: Severity,
    message: &str,
    fields: Vec<(String, Value)>,
) {
    let corr = journal.next_correlation();
    journal.emit(k, severity, crate::supervise::SELF_NAME, &corr, message, fields);
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::kernel::Errno;
    use crate::testkernel::FakeKernel;
    use rivet_config::Document;

    /// The board's own `/proc/cpuinfo` tail, read off a Raspberry Pi 4 on
    /// 2026-08-26.
    ///
    /// **A captured fixture rather than an invented one** — `AGENTS.md` §4, and
    /// the specific reason is 2026-08-26's `from_mountinfo` defect: a fixture
    /// invented from a guess at a format described a machine that does not
    /// exist, and the test passed against it for a day.
    const PI_CPUINFO: &str = "\
processor\t: 3
BogoMIPS\t: 108.00
Features\t: fp asimd evtstrm crc32 cpuid
CPU implementer\t: 0x41
Hardware\t: BCM2835
Revision\t: c03111
Serial\t\t: 10000000a1b2c3d4
Model\t\t: Raspberry Pi 4 Model B Rev 1.1
";

    /// Every record that reached the sink, flattened. The same helper
    /// `crate::etc`'s tests use, and it is duplicated rather than shared for
    /// the reason a four-line test helper usually is: making it public would
    /// put a testing convenience on `testkernel`'s surface.
    fn console(k: &FakeKernel) -> String {
        k.records
            .iter()
            .map(|r| format!("{} {} {:?}", r.sev, r.msg, r.fields))
            .collect::<Vec<_>>()
            .join("
")
    }

    fn journal() -> Journal {
        Journal::new("test-boot".to_string(), 0, crate::supervise::SELF_NAME.to_string())
    }

    fn with_cpuinfo(text: &str) -> FakeKernel {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            SERIAL_SOURCE.to_string(),
            crate::testkernel::Script::always(Ok(text.to_string())),
        );
        k
    }

    // ---- reading the serial ------------------------------------------------

    /// The real file, from the real board.
    #[test]
    fn the_serial_is_read_out_of_the_boards_own_cpuinfo() {
        assert_eq!(serial_in(PI_CPUINFO), Some("10000000a1b2c3d4"));
    }

    /// A platform that reports no serial is not a platform with a blank one.
    #[test]
    fn cpuinfo_without_a_serial_yields_nothing() {
        assert_eq!(serial_in("processor\t: 0\nBogoMIPS\t: 108.00\n"), None);
        assert_eq!(serial_in(""), None);
    }

    // ---- what the machine ends up called -----------------------------------

    /// The operator's name wins, and nothing is read off the machine to get it.
    #[test]
    fn a_document_that_names_the_machine_is_the_whole_of_the_answer() {
        let doc = Document::parse("version = 1\nhostname = \"kitchen-pi\"\n").expect("valid");
        let mut k = with_cpuinfo(PI_CPUINFO);
        let naming = name_this_machine(&mut k, &mut journal(), &doc);
        assert_eq!(
            naming.hostname().map(Hostname::as_str),
            Some("kitchen-pi")
        );
        assert!(matches!(naming, Naming::Stated(_)));
        assert_eq!(k.hostname.as_deref(), Some("kitchen-pi"));
    }

    /// **The ruling: absent is a derived name, not silence.** A machine nobody
    /// has named is still a machine somebody has to reach.
    #[test]
    fn a_document_that_names_nothing_makes_the_machine_name_itself() {
        let doc = Document::parse("version = 1\n").expect("valid");
        let mut k = with_cpuinfo(PI_CPUINFO);
        let naming = name_this_machine(&mut k, &mut journal(), &doc);
        let name = naming.hostname().expect("a derived name").as_str().to_owned();
        assert!(matches!(naming, Naming::Derived(_)));
        assert!(name.starts_with(Hostname::DERIVED_PREFIX), "{name}");
        assert_eq!(k.hostname.as_deref(), Some(name.as_str()));
    }

    /// Stable across boots, because a name that changed every boot would be
    /// worse than none — mDNS caches and so do people.
    #[test]
    fn the_same_machine_names_itself_the_same_thing_every_boot() {
        let doc = Document::parse("version = 1\n").expect("valid");
        let first = name_this_machine(&mut with_cpuinfo(PI_CPUINFO), &mut journal(), &doc);
        let again = name_this_machine(&mut with_cpuinfo(PI_CPUINFO), &mut journal(), &doc);
        assert_eq!(first, again);
    }

    /// **Distinct, which is the property that matters more.** Two machines
    /// answering to one `.local` name is a network that has learned a lie.
    #[test]
    fn two_machines_do_not_name_themselves_the_same_thing() {
        let doc = Document::parse("version = 1\n").expect("valid");
        let other = PI_CPUINFO.replace("10000000a1b2c3d4", "10000000deadbeef");
        let a = name_this_machine(&mut with_cpuinfo(PI_CPUINFO), &mut journal(), &doc);
        let b = name_this_machine(&mut with_cpuinfo(&other), &mut journal(), &doc);
        assert_ne!(a, b);
    }

    /// **The serial does not reach the wire.** The name ends up on a multicast
    /// group every device on the link can hear.
    #[test]
    fn the_hardware_serial_is_not_in_the_name_it_derives() {
        let doc = Document::parse("version = 1\n").expect("valid");
        let naming = name_this_machine(&mut with_cpuinfo(PI_CPUINFO), &mut journal(), &doc);
        let name = naming.hostname().expect("a name").as_str().to_owned();
        assert!(!name.contains("a1b2c3d4"), "{name}");
        assert!(!name.contains("10000000a1b2c3d4"), "{name}");
    }

    /// **A machine that cannot distinguish itself keeps `(none)`**, and says
    /// so. Guessing here does not merely produce a wrong name — it produces a
    /// colliding one.
    #[test]
    fn a_machine_with_no_serial_stays_nameless_and_says_why() {
        let doc = Document::parse("version = 1\n").expect("valid");
        let mut k = with_cpuinfo("processor\t: 0\n");
        let mut j = journal();
        let naming = name_this_machine(&mut k, &mut j, &doc);
        assert_eq!(naming, Naming::Nameless);
        assert_eq!(k.hostname, None, "it named a machine it could not identify");
        let said = console(&k);
        assert!(said.contains("no name"), "{said}");
        assert!(said.contains("hostname"), "{said}");
    }

    /// An unreadable `/proc/cpuinfo` is the same case and is reported with what
    /// the system said.
    #[test]
    fn an_unreadable_cpuinfo_is_reported_rather_than_worked_around() {
        let doc = Document::parse("version = 1\n").expect("valid");
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            SERIAL_SOURCE.to_string(),
            crate::testkernel::Script::always(Err(Errno::ENOENT)),
        );
        let mut j = journal();
        assert_eq!(name_this_machine(&mut k, &mut j, &doc), Naming::Nameless);
        // The errno is rendered as the system renders it, not as a symbol.
        assert!(console(&k).contains("errno 2"), "{}", console(&k));
    }

    /// **A kernel that refuses the name is survivable and loud.** `/etc/hosts`
    /// will still carry it, so the machine resolves a name `uname` does not
    /// report — two answers to one question, which is what this ruling exists
    /// to prevent, so it is `Warn`.
    #[test]
    fn a_kernel_that_refuses_the_name_does_not_stop_the_boot_and_says_what_it_costs() {
        let doc = Document::parse("version = 1\nhostname = \"kitchen-pi\"\n").expect("valid");
        let mut k = with_cpuinfo(PI_CPUINFO);
        k.set_hostname_result = Err(Errno::EPERM);
        let mut j = journal();
        let naming = name_this_machine(&mut k, &mut j, &doc);
        // The name still applies — `/etc/hosts` gets it even though the kernel
        // did not.
        assert_eq!(naming.hostname().map(Hostname::as_str), Some("kitchen-pi"));
        let said = console(&k);
        assert!(said.contains("disagree"), "{said}");
        assert!(said.contains("errno 1"), "{said}");
    }
}
