//! Rivet's CLI — and, before it is a CLI, the binary `/etc/shells` names.
//!
//! `ADR-014` §2 lists `cmd/rivet` as *the CLI*; `ADR-006` §3 makes it one
//! adapter over `rivet-core` among several co-equal ones, containing no logic
//! of its own.
//!
//! ## Why it exists now, ahead of most of what it will administer
//!
//! Measured on a Raspberry Pi 4:
//!
//! > **The account is rejected before authentication if the login shell is
//! > not listed in `/etc/shells`.** … The key was valid and correctly installed;
//! > the connection never reached authentication at all.
//!
//! and the fix it names is *"to list Rivet's own CLI in a generated
//! `/etc/shells`"*. `cmd/rivet-image` refuses to name a binary the image does
//! not contain, so until this crate existed the constant was `None`, the file
//! would have permitted nothing, and **no remote login could succeed on a Rivet
//! card at all.** This binary is what that constant points at.
//!
//! ## Which fraction of `ADR-006` this is
//!
//! Stated as a list, because `AGENTS.md` §6 wants what is unproven said out
//! loud and `ADR-006` §9's recorded risk is *over-building an administrative
//! surface before there is anything to administer*.
//!
//! **Built:**
//!
//! | | Where |
//! | :-- | :-- |
//! | The login-shell contract — `argv[0]`'s dash, `-c`, and silence on stdout | [`invocation`] |
//! | A typed, coded, catalogued refusal surface | [`refusal`] — `ADR-006` §4.1, §4.2 |
//! | Structured value primary, text rendered *from* it, `--output json\|text`, TTY-detected | [`value`], [`cli`] — `ADR-006` §4 |
//! | `ADR-006` §6.4's closed verb vocabulary, read from `rivet_core::Verb` | [`cli`] |
//! | **The operation catalogue**, generated from the core's types and published by `--version` | `rivet_core::operation` — `ADR-006` §7 |
//! | **`rivet schema`**, generated from `rivet-config`'s types | `rivet_config::schema` — `ADR-006` §7, `ADR-007` §3.2 |
//! | **`rivet explain <path>`**, the first administrative operation | `rivet_core::operation::explain` — `ADR-007` §3.6.1 item 3 |
//! | **`rivet apply trial-boot <slot>`**, the first operation that *changes* a machine | `rivet_core::operation::trial` — `ADR-018` §5, `ADR-028` §4 |
//! | **`rivet apply power-off\|restart\|trial-restart`**, the only way anything on a Rivet board can stop it | `rivet_core::operation::stop`, [`shutdown`] — `ADR-017` §3.1.5 item 6, §3.1.10 |
//! | **The first domain port, and a syscall seam of this crate's own** | [`boot`], [`syscall`] — `AGENTS.md` §3.1, `ADR-009` §3.4 |
//! | **`rivet measure hash <path>`**, `ADR-029` §7.2's outstanding measurement | [`measure`] — and it is **not a verb**; see that module |
//! | `--help`, `--version` | this file |
//!
//! ~~**Not built:** every administrative operation. All nine verbs answer
//! `E_UNIMPLEMENTED`.~~ **Amended 2026-08-23.** One verb — `explain` — is
//! implemented, in `rivet-core` where `ADR-006` §3 requires it. The other
//! eight still answer `E_UNIMPLEMENTED`, and they now do so because
//! `rivet_core::operation::CATALOGUE` has no entry for them rather than
//! because this file says so.
//!
//! **Still not built, and each is somebody's next change rather than an
//! oversight:**
//!
//! * ~~**Every operation that changes the machine.**~~ **Amended again, later on
//!   2026-08-23: one exists** — `rivet apply trial-boot <slot>`, which arms
//!   `ADR-018` §5's one-shot trial boot. It is in `rivet-core` where
//!   `ADR-006` §3 requires it, it requires confirmation (`ADR-012` §7.1), and it
//!   does **not** reboot. What is still not built is the framework around it:
//!   `ADR-006` §6's desired-state plan and apply, the document as an argument,
//!   and §6.5's rule that a mutating operation is a *type that computes its diff
//!   separately from applying it*. That operation supplies the dry-run property
//!   **by hand, for itself**, and says so rather than looking like the framework
//!   arrived (`AGENTS.md` §3).
//! * **Every operation over the configuration document.** `rivet get
//!   network.eth0.address` is `ADR-006` §6.4's shape and it cannot be written
//!   in `rivet-core` today: `ADR-014` §3.2 makes `rivet-config` depend on
//!   `rivet-core` **permanently**, so `Document` is not a type the core can
//!   name. The ADR anticipates this — *"if a genuine need ever appears for core
//!   to hold a document, the answer is to move the shared types down"* — and
//!   that move is the maintainer's call, not an agent's.
//! * **Every operation over observed state.** `ADR-014` §2's
//!   `rivet-adapter-system` does not exist, and the kernel is the authority
//!   for what this machine is doing (`AGENTS.md` §5).
//! * **An interactive prompt.** Deliberately, and permanently: `ADR-005` §5.1
//!   rules `nushell` the interactive login shell and **explicitly rejects**
//!   making `rivet` the shell with no general shell at all. A prompt here
//!   would foreclose a ruled decision by satisfying its requirement badly.
//! * **Executing any program.** See [`invocation`]. `scp` and `sftp` therefore
//!   do not work against a Rivet host, which is `PLAN.md` §4.26 and not this
//!   crate's to fix.
//!
//! ## Two things it does not do that a shell does
//!
//! ~~It does not read a configuration file, and it does not touch the network,
//! the filesystem or any device. The only I/O in this crate is writing to the
//! two output streams.~~
//!
//! **Half of that stopped being true on 2026-08-23**, and it is struck rather
//! than edited because `AGENTS.md` §7 is about exactly this sentence outliving
//! its subject. What is still true, and what is not:
//!
//! * **It still reads no configuration file**, and still touches no network.
//! * **It reads a block device end to end**, when — and only when — an operator
//!   runs `rivet measure hash <path>`. Read-only, one open and a stream of
//!   `read(2)`, no `unsafe` beyond the clock; see [`measure`] for why a benchmark
//!   lives in the login shell, which is that on a Rivet board nothing else can
//!   be run at all.
//! * **It mounts a FAT filesystem and rewrites one file on it**, when — and only
//!   when — an operator confirms `rivet apply trial-boot <slot>`. `ADR-028` §4
//!   makes an authenticated principal the source of that authority and `ADR-006`
//!   §5 makes this binary the surface they reach it through, so the I/O arrived
//!   with the first operation that changes a machine rather than by drift. It
//!   lives behind two ports and one wall — [`boot`] and [`syscall`] — and no
//!   other code path in this crate can reach a syscall.
//!
//! **`main` is still short and everything below it is still testable on any
//! machine, including the Windows host this is written on.** That is now a
//! property of the seams rather than of there being no I/O: [`run`] takes the
//! machine as a `&mut dyn Machine`, every test drives a fake, and on a platform
//! that is not Linux the real one **refuses** rather than being absent
//! (`AGENTS.md` §3). `explain`, `schema` and `--version` reach no machine at all.

mod press;
mod device;
mod document;
mod memory;
mod network;
mod process;
mod cpu;
mod slot;
mod thermal;
/// `ADR-028` §5.5.6's `get trust` — reading `/system/trust` back.
mod trust;
mod uptime;
mod boot;
mod cli;
mod deploy;
mod edit;
mod follow;
mod invocation;
mod handler;
// `json` moved to `rivet_core::json` on 2026-08-30 — `ADR-035` §8A.1.2 needs it
// in PID 1, which cannot depend on a binary crate, and two parsers for untrusted
// input is what `ADR-011` forbids.
use rivet_core::json;
mod journal;
mod mcp;
mod measure;
mod refusal;
mod reload;
mod shutdown;
mod status;
mod storage;
// R-168: the zone names THIS machine's own image can resolve, folded into
// `get schema`'s `timezone` property by `boot::PiMachine::configuration_schema`.
// Its own module documentation says why this is not in `rivet-config`.
mod zoneinfo;
#[cfg(target_os = "linux")]
mod syscall;
// `value`'s renderers became the `rivetos-render` crate on 2026-08-30 —
// `ADR-014` §2 had listed it and `ADR-035` §8A.1.2 is what finally needed it.
use rivetos_render as value;

use std::io::IsTerminal as _;
use std::process::ExitCode;

use rivet_core::operation;
use rivet_core::operation::Board;

use cli::{Output, Parsed, Request};
use invocation::{INSTALL_PATH, Invocation};
use refusal::{Code, Refusal};
use value::Value;

/// What `run` produced: the two streams and the exit status, as data.
///
/// Separated so the **stdout invariant is assertable**. Measured on hardware
/// what a stray byte on stdout costs during an `scp`: the greeting was
/// read as an SFTP packet length and the client failed with `Received message
/// too long 1717924466`, an error naming neither the shell nor the greeting.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Streams {
    stdout: String,
    stderr: String,
    code: u8,
}

fn main() -> ExitCode {
    // `argv[0]` is load-bearing here in a way it is in no other Rivet binary:
    // its leading `-` is how whatever exec'd it says "interactive login".
    // An absent `argv[0]` is not a login.
    let argv0 = std::env::args().next().unwrap_or_default();
    let args: Vec<String> = std::env::args().skip(1).collect();

    // The composition root's one act: choose the machine, and hand it inward.
    // `ADR-006` §3 — the adapter wires, the core decides.
    let mut machine = boot::machine();
    let streams = run(
        &argv0,
        &args,
        std::io::stdout().is_terminal(),
        machine.as_mut(),
    );
    if !streams.stdout.is_empty() {
        println!("{}", streams.stdout);
    }
    if !streams.stderr.is_empty() {
        eprintln!("{}", streams.stderr);
    }
    ExitCode::from(streams.code)
}

/// The whole program, over borrowed inputs so every path is drivable from a
/// test on any platform.
///
/// **`machine` is borrowed rather than constructed here**, for the same reason
/// `stdout_is_terminal` is a `bool` rather than a call to `IsTerminal`: taking
/// the answer is what makes both branches testable on a build machine. Every
/// test below drives a fake, and no test in this crate touches a card.
fn run(
    argv0: &str,
    args: &[String],
    stdout_is_terminal: bool,
    // One object, both ports. `Board` is `Machine + Observed` with a blanket
    // impl, so the composition root keeps handing this binary a single machine
    // and the operations behind `perform` still see only the port each needs.
    machine: &mut dyn Board,
) -> Streams {
    run_with_ports(
        argv0,
        args,
        stdout_is_terminal,
        machine,
        &mut follow::LiveConsole::new(),
        &mut journal::ActiveFile::new(),
    )
}

/// The same, over the two seams a `--follow` needs.
///
/// **They are separate parameters rather than part of `machine`** because they
/// are not the machine: a console is where this process's output goes and a
/// journal tail is a file offset, and `AGENTS.md` §3.1 puts both on this side of
/// the boundary. `run` supplies the real ones; every test drives fakes, so a
/// stream that takes five minutes on a board takes microseconds here and its
/// bounds are provable rather than asserted.
fn run_with_ports(
    argv0: &str,
    args: &[String],
    stdout_is_terminal: bool,
    machine: &mut dyn Board,
    console: &mut dyn follow::Console,
    journal: &mut dyn journal::Appending,
) -> Streams {
    // The output mode has to be known before anything can be *rendered*,
    // including a refusal — and a refusal can happen before the arguments that
    // would have chosen a mode are parsed. Auto-resolution is the answer: a
    // refusal from `classify` renders the way the caller's stdout implies.
    let fallback = Output::Auto.resolve(stdout_is_terminal);

    let invocation = match invocation::classify(argv0, args) {
        Ok(invocation) => invocation,
        Err(refusal) => return refuse(&refusal, fallback),
    };

    let args = match invocation {
        Invocation::InteractiveLogin => return refuse(&no_interactive_shell(), fallback),
        Invocation::Arguments(args) => args,
        Invocation::Command(command) => {
            let words = match invocation::words(&command) {
                Ok(words) => words,
                Err(refusal) => return refuse(&refusal, fallback),
            };
            match invocation::self_invocation(&words) {
                Ok(rest) => rest,
                Err(refusal) => return refuse(&refusal, fallback),
            }
        }
    };

    let Parsed { output, request } = match cli::parse(&args) {
        Ok(parsed) => parsed,
        Err(refusal) => return refuse(&refusal, fallback),
    };
    let output = output.resolve(stdout_is_terminal);

    match request {
        Request::Help => Streams {
            stdout: USAGE.to_string(),
            stderr: String::new(),
            code: 0,
        },
        Request::Version => produce(&version(), output),

        // `ADR-029` §7.2's one outstanding measurement. Not an operation and not
        // in the catalogue — see [`measure`] for why it is here at all, which is
        // that this binary is the login shell and nothing else can be run on a
        // Rivet board.
        Request::Handler { name, rest } => match name {
            handler::MEASURE => match measure::perform(&rest) {
                Ok(result) => produce(&result, output),
                Err(refusal) => refuse(&refusal, output),
            },
            // **`ADR-032` §4's adapter.** It reads and writes the process's own
            // streams rather than going through `output` or `console`, and that
            // is deliberate: those two render for a human — `--output text` at a
            // terminal, notes to stderr — and a protocol that went through
            // either could be decorated by a future change to how Rivet talks
            // to people. JSON-RPC is not talk.
            handler::MCP => {
                if let Some(word) = rest.first() {
                    return refuse(
                        &Refusal::about(
                            Code::ARGUMENTS,
                            word,
                            "is not a word `rivet mcp` takes: it takes none. It reads JSON-RPC from stdin and writes replies to stdout (ADR-006 §3.3).",
                        ),
                        output,
                    );
                }
                let mut stdin = std::io::stdin().lock();
                let mut stdout = std::io::stdout().lock();
                match mcp::serve(&mut stdin, &mut stdout, machine) {
                    // The session ended, which is how this always ends.
                    Ok(()) => Streams {
                        stdout: String::new(),
                        stderr: String::new(),
                        code: 0,
                    },
                    Err(e) => refuse(
                        &Refusal::about(
                            Code::ARGUMENTS,
                            "mcp",
                            &format!("the session ended on an I/O failure: {e}"),
                        ),
                        output,
                    ),
                }
            }
            // Unreachable through `crate::cli`, which only builds a `Handler`
            // for a name `handler::HANDLERS` carries. Written as a refusal
            // rather than an `unreachable!` for `rivet_core::operation::perform`'s
            // reason: a panic in the login shell is a session that dies
            // without saying why, and if the table and this match ever disagree
            // the honest answer is still that this build does not implement it.
            other => refuse(
                &Refusal::about(
                    Code::ARGUMENTS,
                    other,
                    "is listed as an internal handler (ADR-032 §4.2) and this build has no  \
                     implementation behind it; that is a defect. `rivet --version` publishes  \
                     the table.",
                ),
                output,
            ),
        },
        // **The whole of this adapter's administrative logic.** The words after
        // the verb are handed to the core, which binds them to the operation's
        // declared inputs, performs it, and returns a typed result or a typed
        // error. `ADR-006` §3: the CLI parses arguments and renders results.
        Request::Verb { verb, rest } => {
            // `--follow` is read here rather than by the operation, because it
            // is not one of the operation's arguments: it says how the answer
            // is *delivered*, and the operation still answers once (`ADR-022`
            // §9.1, [`follow`]). Everything it does not claim is passed through
            // untouched.
            let (following, rest) = match follow::extract(verb == rivet_core::Verb::Get, &rest) {
                Ok(split) => split,
                Err(refusal) => return refuse(&refusal, output),
            };
            if let Some(options) = following {
                return match follow::stream(&options, console, journal) {
                    // The stream itself has already been written, line by line,
                    // through `console`. What comes back is the summary, and it
                    // goes where every other sentence that is not a record went.
                    Ok(summary) => {
                        console.note(&render(&summary, output));
                        Streams {
                            stdout: String::new(),
                            stderr: String::new(),
                            code: 0,
                        }
                    }
                    Err(refusal) => refuse(&refusal, output),
                };
            }

            let borrowed: Vec<&str> = rest.iter().map(String::as_str).collect();
            match operation::perform(verb, &borrowed, machine) {
                Ok(result) => produce(&result, output),
                Err(error) => refuse(&Refusal::from(error), output),
            }
        }
    }
}

/// Render a value. **Always to stdout**, and only ever a value a command
/// actually produced.
fn produce(value: &Value, output: Output) -> Streams {
    Streams {
        stdout: render(value, output),
        stderr: String::new(),
        code: 0,
    }
}

/// Render a refusal. **Always to stderr, never to stdout** — see [`Streams`].
fn refuse(refusal: &Refusal, output: Output) -> Streams {
    Streams {
        stdout: String::new(),
        stderr: render(&refusal.value(), output),
        code: refusal.exit_code(),
    }
}

/// The one place that chooses between the two readings of a value.
///
/// `ADR-006` §4: both are readers of the same structure, so there is nothing a
/// caller can learn from one that is absent from the other.
fn render(value: &Value, output: Output) -> String {
    match output {
        Output::Json => value::json(value),
        // `Auto` cannot reach here — `run` resolves it against the caller's
        // stdout before rendering anything. Rendering it as text rather than
        // adding an unreachable branch keeps the function total.
        //
        // **Journal records are rendered as `ADR-022` §2's view rather than as
        // the lines they are**, and that is this function's job rather than the
        // operation's. §2 is explicit — *"a log entry is a structured record;
        // the text an operator reads is a view of it"* — so the record stays
        // authoritative in the value and the human reading is made here, by an
        // adapter, out of `rivet_core::journal::Record::to_text`. The
        // rendering itself is the domain's; only the decision to use it is
        // this crate's, which is `ADR-006` §3's "the CLI renders results".
        Output::Text | Output::Auto => value::text(&journal::as_text_view(value)),
    }
}

/// `--version`, and **the surface describing itself** — `ADR-006` §7.
///
/// ~~`operations` is an empty list and that is the honest catalogue.~~
/// **Amended 2026-08-23.** It is `rivet_core::operation::catalogue()`, which
/// is generated from `CATALOGUE`, which is generated from the `const OPERATION`
/// each operation's own module declares. `ADR-006` §7 requires exactly that,
/// and requires it because a hand-maintained catalogue *"will be wrong within a
/// month and worse than absent, because it will be trusted."* There is no third
/// place the list of operations is written down, and
/// `the_published_catalogue_is_the_one_the_core_performs` is what keeps it that
/// way.
///
/// `errors` is generated too — from [`refusal::Code::all`], which is the union
/// of `rivet_core::ErrorCode::ALL` and this crate's own two login-shell codes,
/// each half read from its own definition rather than copied. `ADR-006` §4.1
/// makes the codes part of the interface contract, and a contract an agent has
/// to discover by provoking each failure is not one it can read.
fn version() -> Value {
    Value::object(vec![
        ("name", Value::str("rivet")),
        ("version", Value::str(env!("CARGO_PKG_VERSION"))),
        ("login-shell", Value::str(INSTALL_PATH)),
        (
            "schema-version",
            Value::Integer(i64::from(rivet_config::SCHEMA_VERSION.get())),
        ),
        ("operations", operation::catalogue()),
        // **`ADR-032` §4.2's internal handlers**, beside the operations and not
        // inside them: an operation is `rivet-core`'s and reachable from every
        // adapter, a handler is this login shell's and reachable only here.
        // Published because until 2026-08-26 `measure` appeared NOWHERE in
        // anything this machine says about itself — a catalogue that is
        // silently incomplete is `ADR-006` §7's "trusted and wrong" with a
        // quieter failure.
        ("handlers", handler::value()),
        (
            "errors",
            Value::strings(Code::all().iter().map(|c| c.as_str())),
        ),
    ])
}

/// The refusal a human meets if anything ever execs this as a login shell.
///
/// **Nothing in the current image does** — `ADR-035` left the control plane
/// as the only way in. It is kept rather than deleted because a board flashed
/// from an older card can still land here, and a refusal that answered with a
/// panic would tell that reader nothing.
///
/// It has to answer the question the reader is about to ask — *why is there no
/// prompt* — and point at the ruling rather than reading as a defect.
fn no_interactive_shell() -> Refusal {
    Refusal::new(
        Code::NoInteractiveShell,
        format!(
            "this image has no interactive shell. rivet is not a shell; nushell is the ruled \
             interactive login shell (ADR-005 §5.1) and this image does not contain it. {}",
            rivet_core::control::ASK_A_BOARD
        ),
    )
}

/// `--help`. Plain text on stdout, and the only stdout this binary produces that
/// is not a rendered [`Value`] — a usage message is prose by nature and
/// pretending otherwise would be `ADR-006` §4 applied where it does not fit.
const USAGE: &str = "\
rivet — Rivet's administration CLI, and the binary /etc/shells names.

    rivet [--output json|text] <verb> [...]
    rivet get schema
    rivet measure hash <path>
    rivet --version
    rivet --help

Verbs (ADR-006 §6.4's closed set): get, set, list, add, remove, apply, plan,
export, explain. THIS BUILD IMPLEMENTS THREE OF THEM.
`get` reads TWENTY subjects; `apply` takes TWENTY-NINE subjects.
`apply restart` restarts the MACHINE; `apply service-restart <name>` restarts ONE SERVICE.
START WITH `rivet get catalogue` IF YOU DO NOT KNOW WHAT THIS MACHINE CAN DO: it is
the catalogue above, over the wire, and it is the one command that needs no shell.

THE LIST BELOW IS NOT ALL OF THEM AND IS NOT THE AUTHORITY. It shows the few
that need prose to use safely. `rivet --version` publishes the operation
catalogue, generated from the types; that is the authority for what exists, and
`rivet get schema` is the authority for the document. Ask one of those rather
than this text.

    rivet get log [boot] [limit] [--follow [--for <seconds>|forever]]
                             this machine's journal (ADR-022 §9) — what it did,
                             in the order it did it. `boot` is `latest` (the
                             default), `previous` (the one before this — what to
                             ask after a board came back), `all`, or a boot
                             identifier. It cannot be selected by TIME: a Pi has
                             no RTC, so every boot starts at the epoch until
                             something corrects it (ADR-021). Records come back
                             as the lines the machine wrote; in nushell:
                               rivet get log | from json | get records
                                 | each { from json } | where sev == error
                             `limit` DEFAULTS TO 200 — the last 200 records, not
                             all of them (ADR-022 §9.6). The word `all` is how to
                             ask for everything the journal still holds.
                             `--follow` streams instead of answering: one record
                             per line as it is written, stdout carrying nothing
                             else, until `--for` runs out (300 seconds by
                             default, or `forever`). In nushell:
                               rivet get log latest 20 --follow --for 60
                                 | each { from json }
    rivet get storage      how full every mounted filesystem is (ADR-005
                             §4.1.3) — what it is, where it is mounted, its type,
                             total/used/available bytes, and which of Rivet's
                             own locations it holds. NOTHING IS FILTERED; select
                             in the shell:
                               rivet get storage | from json | get filesystems
                                 | where mount-point == /state
                             /system is a DIRECTORY in the root filesystem and
                             not a mount point, so look for it in `holds` rather
                             than as a row of its own. capacity-percent and the
                             inode fields are ABSENT on a filesystem that has no
                             such number — /proc is one on every machine — so
                             `select` them with a trailing ?:
                               ... | select mount-point capacity-percent? holds?
    rivet explain <path>   what generates a path, and what to change instead
    rivet apply trial-boot <a|b> [confirm]
                             arm a one-shot trial boot of the other A/B slot
    rivet apply power-off [confirm]
                             stop the machine and leave it off
    rivet apply restart [confirm]
                             stop the machine and bring it back on the DEFAULT
                             slot. An armed trial boot is NOT taken by this
    rivet apply trial-restart [confirm]
                             stop the machine and bring it back ONCE in the slot
                             a trial boot armed (ADR-018 §5.1). This is the only
                             restart that carries the firmware's one-shot flag
    rivet apply service-restart <name> [confirm]
                             restart ONE SERVICE and nothing else — NOT the
                             machine. This is how a deployed application starts
                             running the bytes you shipped (ADR-017 §6).
                             It may only ask the supervisor to CONVERGE toward
                             the desired-state document: it restarts a service
                             the document enables, and starts an enabled one
                             that is idle, failed, backing off or done. It
                             REFUSES to start a service the document disables.
                             To change what this machine should be running,
                             edit the document:
                               rivet apply edit /state/config/machine.toml ...
                               rivet apply reload confirm
                             The plan comes from the supervisor itself, so it
                             names the state your service is actually in. If the
                             service you name is the one carrying this request —
                             rivet-mcpd — the answer may not arrive; a dropped
                             connection is normal there and `rivet get status`
                             is what says whether it worked.
    rivet apply service-stop <name> [confirm]
                             stop ONE APPLICATION and keep it stopped — it is
                             not restarted as a crash — without uninstalling it.
                             Its files and data under /state/app are untouched
                             and `enabled` in the document is not changed. The
                             stop lasts until `service-start`, or until the next
                             boot, which returns the machine to its document.
                             REFUSED for rivet-mcpd and rivet-netd, which carry
                             the way in, and for every other image service.
    rivet apply service-start <name> [confirm]
                             release a `service-stop`; the application then runs
                             again if the document enables it. Refused for a
                             service that was not stopped that way.
    rivet apply clock-floor-reset [confirm]
                             make the running clock this machine's clock floor
                             (ADR-021 §3): the time the next boot restores the
                             clock to at the least. The floor only rises on its
                             own, so a clock once stepped to a wrong future is
                             restored to it at every boot until this lowers it.
                             The plan names the floor now and the running clock
                             that would replace it. Correct the running clock
                             first if it is wrong: the floor follows it.
    rivet apply uninstall-library <product|reference> [confirm]
                             remove an installed LIBRARY. A product name removes
                             every build of that product under /state/lib; a
                             full reference <name>-<version>-<64 hex> removes
                             exactly one. REFUSED, removing nothing, if an
                             installed application names a build it would
                             remove in its manifest's `library` closure, or if
                             an application's manifest cannot be read. An
                             application that only accepts the library's class
                             is named in the plan and loses the addon at its
                             next start: nothing running is restarted, and the
                             supervisor recomposes every service's library path
                             without a reboot.

Every other verb refuses with E_UNIMPLEMENTED. `rivet --version` publishes the
operation catalogue, generated from the types, which is the authority for what
exists; `rivet schema` publishes the configuration document's JSON Schema.

schema and measure are NOT verbs and are not in ADR-006 §6.4's closed set; they
are not in the operation catalogue either, and Verb::parse refuses both.

    rivet measure hash <path>
                             hash a block device or file with BLAKE3 and SHA-256
                             in one streaming pass, reporting bytes, elapsed
                             milliseconds, MB/s and both digests. It changes
                             nothing. This exists to settle ADR-029 §7.2 and
                             ADR-015 §4.2 on real hardware:
                               rivet measure hash /dev/mmcblk0p3
                             The per-algorithm times exclude the card — read time
                             is reported separately — and MB means 10^6 bytes.

apply changes the machine, so it confirms first (ADR-012 §7.1): without the
final word `confirm` it reads the machine, checks the safety interlocks, changes
nothing, and refuses with the plan. A misspelt `confirm` is refused rather than
read as absence.

trial-boot NEVER reboots — arming and restarting are two acts (ADR-018 §5), and
it says which restart the arming needs. The three stop subjects never arm.
So a trial boot on this hardware is two commands, in this order:

    rivet apply trial-boot b confirm      arm it
    rivet apply trial-restart confirm     take it

The stop subjects ask pid 1, which owns shutdown (ADR-017 §3.1.5 item 6): it
stops the supervisor, waits for it, syncs the filesystems, and only then asks
the kernel. So the machine has NOT stopped when this command answers, and the
answer races the shutdown of whatever channel carried the request — ADR-035's
control plane, or a session on the machine — so the client may print the answer
and exit 0, or may report a dropped connection, and both are normal. Read the
console for what happened after it.

rivet REFUSES to signal pid 1 unless /proc/1/comm is exactly `rivet-initd`,
and refuses if it cannot read it. On a development host pid 1 is that host's own
init, and this command would otherwise stop the machine you are working on.

Output is structured first (ADR-006 §4). A terminal gets the human rendering
and a pipe gets JSON; --output forces either.

From a workstation (ADR-035 — the control plane is the only transport):
  cargo rivet run --host <host>:7777 --identity <dir> --pin <hex> -- <command>
  cargo rivet run --show -- <command>   prints the tool call, sends nothing

The login-shell behaviour below is DORMANT. Nothing in this image execs rivet
as a shell; it is still described because a card built before ADR-035
behaves this way:
  a bare login          refused, E_NO_INTERACTIVE_SHELL — nushell is the ruled
                        interactive shell and is not in this image
  \"rivet ...\"           runs that command
  \"anything-else\"       refused, E_NOT_A_SHELL — rivet executes no programs,
                        which is also why a remote file copy never worked
                        against a Rivet host (PLAN.md §4.26)

Exit codes: 0 a value; 1 the operation refused; 2 the arguments were not
usable; 126 an interactive session was asked for; 127 -c named a program.";

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use rivet_core::operation::{Machine, MachineError, Observed, Reboot, Stop};
    use rivet_core::state::Slot;
    use rivet_core::{ErrorCode, Verb};

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_string()).collect()
    }

    /// A board that answers: running slot A, and willing to arm slot B.
    ///
    /// Every test in this file drives this rather than a card. `ADR-026` §4 puts
    /// `tryboot` and `/proc/device-tree/chosen/bootloader` in the column QEMU
    /// does not emulate at all, so a green suite here says the CLI's wiring is
    /// right *given a machine that answers as scripted* and says nothing about
    /// whether a board does — `AGENTS.md` §6. `crate::boot`'s tests carry the
    /// stack below this; these carry `argv` to a value and back.
    struct FakeBoard {
        armed: Vec<Slot>,
        /// Every stop this board was **asked** for. A board that was asked to
        /// stop is the whole of what this crate can observe; see
        /// `rivet_core::operation::machine::Machine::request_stop`.
        stopped: Vec<Stop>,
        /// And every stop that was *checked* without being asked for, so the
        /// plan path can be told apart from the confirmed one.
        planned_stops: Vec<Stop>,
        /// **The real install channel over a filesystem in a map.**
        ///
        /// `crate::deploy::Requests` is what a board runs; only the store under
        /// it is faked, because the alternative is a suite that writes to a real
        /// `/state` — which the development host under WSL does not have and the
        /// one machine that does is the maintainer's board (`AGENTS.md` §9).
        installs: crate::deploy::Requests<crate::deploy::Memory>,
        /// **The real editor over a filesystem in a map**, for `installs`'
        /// reason and a sharper one: `apply edit` writes to a path the operator
        /// names, so a suite over the real store could be made to write anywhere
        /// on the development host by a single mistyped literal (`AGENTS.md`
        /// §9).
        edits: crate::edit::Files<crate::edit::Memory>,
        /// Every service this board was asked to restart, so the plan path can
        /// be told apart from the confirmed one. `ADR-017` §6.3.2.
        restarted: Vec<String>,
        /// Every `service-stop` or `service-start` this board was asked to
        /// apply, as `<subject> <service>`. `ADR-017` §6.
        held: Vec<String>,
        /// **Set only by the adapter-to-operation storage test.** `None` is
        /// every other test's case and keeps today's behaviour exactly:
        /// `read_filesystems` falls through to the real `crate::storage::read()`
        /// seam. `Some` lets one test hand this board a `StorageRead` a REAL
        /// `rivet_adapter_system::read` produced over a fake `SystemMedium`,
        /// so the shadowing this fix is about is proven through the actual
        /// adapter conversion and not only through a hand-built fixture.
        filesystems_override: Option<rivet_core::operation::StorageRead>,
    }

    /// The artifact digest every whole-binary install test below asks for.
    ///
    /// **Spelled here rather than in each test**, because since `ADR-052` §7 the
    /// argument to `apply install` is a digest and a test that invented its own
    /// would be asking for something the fake store does not hold.
    const A_DIGEST: &str = "8c4e9b21f0a37d5e6b18c92470fd3a85e1c7b0946d2af38e5710cb639ad84f2c";

    /// A second one, for the test about a request that is already waiting.
    const ANOTHER_DIGEST: &str =
        "1d2e3f405162738495a6b7c8d9eafb0c1d2e3f405162738495a6b7c8d9eafb0c";

    fn a_digest() -> rivet_core::artifact::Digest {
        rivet_core::artifact::Digest::new(A_DIGEST)
            .unwrap_or_else(|| unreachable!("{A_DIGEST} is a digest"))
    }

    fn another_digest() -> rivet_core::artifact::Digest {
        rivet_core::artifact::Digest::new(ANOTHER_DIGEST)
            .unwrap_or_else(|| unreachable!("{ANOTHER_DIGEST} is a digest"))
    }

    impl FakeBoard {
        fn new() -> Self {
            Self {
                armed: Vec::new(),
                stopped: Vec::new(),
                planned_stops: Vec::new(),
                // **The principal this host actually has**, whatever it is, and
                // not one invented for the test: these are whole-binary tests,
                // and pinning a fake session here would make them prove the
                // channel works for a principal `crate::deploy::this_process`
                // may never produce.
                // **Holding the two artifacts these tests install**, since
                // `ADR-052` §7: an install names a digest and a machine whose
                // store is empty refuses every one of them, which would make
                // these tests prove the refusal rather than the channel.
                installs: crate::deploy::Requests::new(
                    crate::deploy::Memory::rivet()
                        .holding(&a_digest(), "photo-index-0.1.0-aarch64-application.rvt")
                        .holding(&another_digest(), "photo-index-0.2.0-aarch64-application.rvt"),
                    crate::deploy::this_process(),
                ),
                edits: crate::edit::Files::new(
                    crate::edit::Memory::holding(
                        FakeBoard::EDITABLE,
                        "# nushell\n$env.config.show_banner = true\n",
                    ),
                    crate::deploy::this_process(),
                ),
                restarted: Vec::new(),
                held: Vec::new(),
                filesystems_override: None,
            }
        }

        /// One file this fake machine holds, so `apply edit` has something to
        /// change in a whole-binary test.
        const EDITABLE: &'static str = "/state/home/root/.config/nushell/config.nu";
    }

    impl Machine for FakeBoard {
        // `ADR-032` §3.2's reload. This fake is the CLI's end-to-end board and
        // answers what the tests here drive; a reload is not one of them, so it
        // refuses rather than pretending (`AGENTS.md` §3).
        fn can_request_reload(&mut self) -> Result<(), rivet_core::operation::MachineError> {
            Err(rivet_core::operation::MachineError::Refused(
                "this fake does not reload".to_string(),
            ))
        }
        fn request_reload(&mut self) -> Result<(), rivet_core::operation::MachineError> {
            self.can_request_reload()
        }

        /// `ADR-017` §6.3.2's service control. **This fake answers rather than
        /// refusing**, because the property the whole-binary tests are for is
        /// that `argv` reaches the operation and a value comes back — and the
        /// refusals live in the supervisor, which no test here runs.
        fn can_request_service_restart(
            &mut self,
            service: &str,
        ) -> Result<rivet_core::operation::Restarting, rivet_core::operation::MachineError> {
            Ok(rivet_core::operation::Restarting {
                service: service.to_string(),
                was: "started".to_string(),
                doing: rivet_core::operation::Doing::Stop,
                reachability: false,
            })
        }

        fn request_service_restart(
            &mut self,
            service: &str,
        ) -> Result<rivet_core::operation::Restarting, rivet_core::operation::MachineError> {
            self.restarted.push(service.to_string());
            self.can_request_service_restart(service)
        }

        /// `ADR-017` §6's held stop. Answers, for `can_request_service_restart`'s
        /// reason: the property here is that `argv` reaches the operation.
        fn can_request_service_hold(
            &mut self,
            kind: rivet_core::operation::ServiceHoldKind,
            service: &str,
        ) -> Result<rivet_core::operation::ServiceHold, rivet_core::operation::MachineError>
        {
            Ok(rivet_core::operation::ServiceHold {
                kind,
                service: service.to_string(),
                was: "started".to_string(),
                doing: rivet_core::operation::service_hold::Doing::Stop,
            })
        }

        fn request_service_hold(
            &mut self,
            kind: rivet_core::operation::ServiceHoldKind,
            service: &str,
        ) -> Result<rivet_core::operation::ServiceHold, rivet_core::operation::MachineError>
        {
            self.held.push(format!("{kind} {service}"));
            self.can_request_service_hold(kind, service)
        }

        fn running_slot(&mut self) -> Result<Slot, MachineError> {
            Ok(Slot::A)
        }

        fn can_arm_trial_boot(&mut self, slot: Slot) -> Result<(), MachineError> {
            if slot == Slot::A {
                return Err(MachineError::Unsafe("that is the running slot".to_string()));
            }
            Ok(())
        }

        fn arm_trial_boot(&mut self, slot: Slot) -> Result<Reboot, MachineError> {
            self.can_arm_trial_boot(slot)?;
            self.armed.push(slot);
            Ok(Reboot::OneShot)
        }

        fn can_request_stop(&mut self, stop: Stop) -> Result<(), MachineError> {
            self.planned_stops.push(stop);
            Ok(())
        }

        /// **Nothing real is signalled.** The interlock that makes that safe on
        /// an actual machine is `crate::shutdown`'s, tested there against this
        /// host's own `/proc`; what is tested here is `argv` reaching the
        /// operation and a value coming back.
        fn request_stop(&mut self, stop: Stop) -> Result<(), MachineError> {
            self.stopped.push(stop);
            Ok(())
        }
    }

    /// The observation half. Both of these are the **real** readers rather
    /// than fakes, which is why they moved together and why the split reads
    /// naturally here: what a development host can honestly answer is exactly
    /// what `Observed` asks.
    impl Observed for FakeBoard {
    fn read_processes(
        &mut self,
    ) -> Result<rivet_core::operation::machine::ProcessRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    fn read_memory(&mut self) -> Result<rivet_core::operation::machine::MemoryRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than answering an idle machine.** Zeroed counters are
    /// what a machine that has run nothing since it booted looks like, and a
    /// load average of zero is what an idle one looks like — both are readings
    /// and this fake has taken neither.
    fn read_cpu(&mut self) -> Result<rivet_core::operation::machine::CpuRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than answering that this machine trusts nobody.** An
    /// empty trust store is a real reading — `ADR-028` §5.5.5's `--no-trust`
    /// image — and this fake has not taken it. The direction is the dangerous
    /// one: *"trusts nobody"* is the answer somebody acts on by re-signing.
    fn read_trust(&mut self) -> Result<rivet_core::operation::machine::TrustRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    fn read_desired_state(&mut self) -> Result<Option<String>, MachineError> {
        Ok(None)
    }
    /// **Refuses rather than answering with unset flags.** A `Throttle` of
    /// all-`false` would assert that this machine has never been under-volted
    /// since it booted, which is the one claim this subject must never make
    /// without a measurement.
    fn read_thermal(
        &mut self,
    ) -> Result<rivet_core::operation::machine::ThermalRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than answering an uptime of zero**, which would say
    /// this machine booted a moment ago.
    fn read_boot(&mut self) -> Result<rivet_core::operation::machine::BootRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than naming a slot**, which is how an update comes to
    /// overwrite the running system.
    fn read_slots(&mut self) -> Result<rivet_core::operation::machine::SlotRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than answering an empty network**, which reads as a
    /// machine with no interfaces at all.
    fn read_network(
        &mut self,
    ) -> Result<rivet_core::operation::machine::NetworkRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than answering that this machine has no block
    /// devices**, which is not a state a machine that booted off one can be in.
    fn read_devices(
        &mut self,
    ) -> Result<rivet_core::operation::machine::DeviceRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
        /// `ADR-032` §3.2's query socket. This fake asks no supervisor and
        /// refuses rather than pretending (`AGENTS.md` §3).
        fn supervisor_status(
            &mut self,
        ) -> Result<rivet_core::operation::Status, rivet_core::operation::MachineError> {
            Err(rivet_core::operation::MachineError::Refused(
                "this fake answers no supervisor".to_string(),
            ))
        }

        /// **The real reader, not a fake**, and it is the one method on this
        /// port where that is the right answer.
        ///
        /// Every other method reaches a card, a device tree or pid 1 — things a
        /// development host does not have and must not be asked for, which is
        /// why this fake exists at all. Reading the journal is
        /// `read_to_string` over absolute paths: there is no platform behind it
        /// to hide, and a fake would be testing the fake.
        ///
        /// What it produces here is the honest answer for this host — no
        /// `/state/log`, so an empty journal that names where it looked. That
        /// is `crate::journal`'s own claim, exercised through the whole binary.
        fn read_journal(&mut self) -> Result<rivet_core::operation::JournalRead, MachineError> {
            crate::journal::read()
        }

        /// **The real reader again, and for the same reason** — `crate::storage`
        /// is `/proc/self/mounts` plus `statvfs(3)`, with no card, no device tree
        /// and no pid 1 behind it, so a fake would be testing the fake.
        ///
        /// It answers differently on the two hosts this suite runs on, and both
        /// answers are honest: under WSL it reports that host's real filesystems,
        /// and on Windows it **refuses** (`crate::storage`'s own module
        /// documentation argues why the two operations differ here). The tests
        /// below therefore accept either and assert what each must contain.
        ///
        /// **Unless `filesystems_override` is set**, which one test uses to hand
        /// this board a reading assembled from the real adapter seam over a
        /// fake medium, rather than this host's own filesystems.
        fn read_filesystems(
            &mut self,
        ) -> Result<rivet_core::operation::StorageRead, MachineError> {
            self.filesystems_override.clone().map_or_else(crate::storage::read, Ok)
        }

        /// **The real one**, because it is the same on every platform and the
        /// tests below assert against the actual document rather than a stand-in.
        fn configuration_schema(&mut self) -> rivet_core::Value {
            rivet_config::schema()
        }
    }

    /// **The real adapter again**, with only the filesystem under it faked.
    ///
    /// ~~`crate::deploy::NoChannel` is what a board runs too — the thing that is
    /// missing is a ruling rather than a platform.~~ **Superseded 2026-08-24
    /// evening**, when the channel was ruled. `crate::deploy::Requests` is now
    /// what a board runs, and a fake store is the smallest thing that could be
    /// substituted: what the tests below exercise is `argv` reaching the
    /// operation, the request being written in the form the supervisor parses,
    /// and the answer coming back — which is the whole of what this crate owns.
    impl rivet_core::operation::Deployer for FakeBoard {
        /// **`crate::repo_delete`'s two, added 2026-08-26.** This fake destroys
        /// nothing and refusing is the honest stand-in (`AGENTS.md` §3): a test
        /// in this module that reached this port has changed what it is about.
        fn can_request_repo_delete(
            &mut self,
            _name: &str,
        ) -> Result<(), rivet_core::operation::MachineError> {
            Err(rivet_core::operation::MachineError::Refused(
                "this fake destroys no repository; an operation that reached this port has \
                 changed"
                    .to_string(),
            ))
        }

        fn request_repo_delete(
            &mut self,
            name: &str,
            _id: &str,
        ) -> Result<(), rivet_core::operation::MachineError> {
            self.can_request_repo_delete(name)
        }

        fn can_request_install(
            &mut self,
            digest: &rivet_core::artifact::Digest,
        ) -> Result<rivet_core::operation::Held, MachineError> {
            self.installs.can_request_install(digest)
        }

        fn request_install(
            &mut self,
            digest: &rivet_core::artifact::Digest,
        ) -> Result<rivet_core::operation::Held, MachineError> {
            self.installs.request_install(digest)
        }

        fn can_request_uninstall(&mut self, name: &str) -> Result<(), MachineError> {
            self.installs.can_request_uninstall(name)
        }

        fn request_uninstall(&mut self, name: &str, purge: bool) -> Result<(), MachineError> {
            self.installs.request_uninstall(name, purge)
        }

        fn can_request_library_uninstall(
            &mut self,
            selector: &rivet_core::uninstall_library::Selector,
        ) -> Result<rivet_core::uninstall_library::Removal, MachineError> {
            self.installs.can_request_library_uninstall(selector)
        }

        fn request_library_uninstall(
            &mut self,
            selector: &rivet_core::uninstall_library::Selector,
        ) -> Result<rivet_core::uninstall_library::Removal, MachineError> {
            self.installs.request_library_uninstall(selector)
        }
    }

    /// **The fourth port, and the real editor over a filesystem in a map** —
    /// `Deployer`'s shape and its argument. What the tests below exercise is
    /// `argv` reaching the operation, the file actually changing, the record
    /// landing in the journal, and the answer coming back.
    impl rivet_core::operation::Editor for FakeBoard {
        fn principal(&mut self) -> rivet_core::deploy::Principal {
            self.edits.principal()
        }

        fn read_file(
            &mut self,
            path: &str,
        ) -> Result<rivet_core::operation::FileRead, MachineError> {
            self.edits.read_file(path)
        }

        fn can_write_file(&mut self, path: &str, contents: &str) -> Result<(), MachineError> {
            self.edits.can_write_file(path, contents)
        }

        fn write_file(
            &mut self,
            write: &rivet_core::operation::Write<'_>,
        ) -> Result<rivet_core::operation::Wrote, MachineError> {
            self.edits.write_file(write)
        }

        // `ADR-006` §8.10's four, added 2026-08-29. Forwarded like the four
        // above rather than stubbed, so the whole-binary tests drive
        // `rivet get file`, `rivet apply remove-file` and the rest end to end
        // through the same map.
        fn stat_path(
            &mut self,
            path: &str,
        ) -> Result<rivet_core::operation::PathStat, MachineError> {
            self.edits.stat_path(path)
        }

        fn resolve_owner(
            &mut self,
            user: Option<&str>,
            group: Option<&str>,
        ) -> Result<rivet_core::operation::OwnerIds, MachineError> {
            self.edits.resolve_owner(user, group)
        }

        fn can_change_path(
            &mut self,
            change: &rivet_core::operation::PathChange<'_>,
        ) -> Result<(), MachineError> {
            self.edits.can_change_path(change)
        }

        fn change_path(
            &mut self,
            change: &rivet_core::operation::PathChange<'_>,
        ) -> Result<rivet_core::operation::Wrote, MachineError> {
            self.edits.change_path(change)
        }
    }

    /// The program, over a board that answers.
    fn run_on_board(argv0: &str, args: &[String], stdout_is_terminal: bool) -> Streams {
        run(argv0, args, stdout_is_terminal, &mut FakeBoard::new())
    }

    /// A pipe, which is what a remote command actually gives this process.
    fn piped(argv0: &str, args: &[&str]) -> Streams {
        run_on_board(argv0, &argv(args), false)
    }

    // -- `--follow`, wired through the ordinary argument path ----------------

    /// A console that records instead of writing, with a clock that only moves
    /// when the follower sleeps.
    #[derive(Default)]
    struct Recorded {
        lines: Vec<String>,
        notes: Vec<String>,
        now_ms: u64,
    }

    impl follow::Console for Recorded {
        fn line(&mut self, text: &str) -> bool {
            self.lines.push(text.to_string());
            true
        }
        fn note(&mut self, text: &str) {
            self.notes.push(text.to_string());
        }
        fn elapsed_ms(&mut self) -> u64 {
            self.now_ms
        }
        fn sleep_ms(&mut self, ms: u64) {
            self.now_ms += ms;
        }
    }

    /// One read of one record, and then a journal that never grows again.
    #[derive(Default)]
    struct OneRecord(bool);

    impl journal::Appending for OneRecord {
        fn appended(&mut self) -> Result<journal::Appended, rivet_core::operation::MachineError> {
            if self.0 {
                return Ok(journal::Appended::default());
            }
            self.0 = true;
            Ok(journal::Appended {
                lines: vec![
                    r#"{"boot":"b1","mono":1.0,"sev":"info","svc":"rivet-sysd","corr":"c","actor":"supervisor","msg":"converged"}"#
                        .to_string(),
                ],
                rotated: false,
                missed: None,
            })
        }
        fn source(&self) -> &str {
            "/state/log/journal.jsonl"
        }
    }

    /// **The wiring, end to end.** `--follow` arrives as an ordinary word after
    /// the verb — `cli::parse` gives everything after `get` to the verb — and
    /// what comes back is a stream on the console rather than a value in
    /// `Streams`.
    #[test]
    fn a_follow_streams_through_the_console_and_returns_nothing_in_stdout() {
        let mut console = Recorded::default();
        let mut tail = OneRecord::default();
        let streams = run_with_ports(
            "rivet",
            &argv(&["get", "log", "latest", "10", "--follow", "--for", "0"]),
            false,
            &mut FakeBoard::new(),
            &mut console,
            &mut tail,
        );

        assert_eq!(console.lines.len(), 1, "{:?}", console.lines);
        assert!(console.lines[0].contains("converged"));
        assert!(
            streams.stdout.is_empty(),
            "a stream is written as it happens, not returned: {:?}",
            streams.stdout
        );
        assert_eq!(streams.code, 0);
        assert!(
            console.notes.iter().any(|n| n.contains("deadline")),
            "the summary must say why it stopped: {:?}",
            console.notes
        );
    }

    /// The summary is JSON when the caller asked for JSON, because it is a
    /// value like every other — `ADR-006` §4.
    #[test]
    fn the_summary_is_rendered_in_the_output_mode_the_caller_chose() {
        let mut console = Recorded::default();
        let mut tail = OneRecord::default();
        run_with_ports(
            "rivet",
            &argv(&["--output", "json", "get", "log", "--follow", "--for", "0"]),
            false,
            &mut FakeBoard::new(),
            &mut console,
            &mut tail,
        );
        let summary = console.notes.last().expect("no summary");
        assert!(summary.starts_with('{'), "{summary}");
        assert!(summary.contains("\"stopped\""), "{summary}");
    }

    /// Refused before anything is read, so no test here touches a real card.
    #[test]
    fn following_a_subject_that_cannot_grow_is_refused_at_the_cli() {
        let out = piped("rivet", &["get", "storage", "--follow"]);
        assert_ne!(out.code, 0);
        assert!(out.stdout.is_empty(), "a refusal never goes to stdout");
        assert!(out.stderr.contains("E_ARGUMENTS"), "{}", out.stderr);
    }

    /// `--follow` is not a global option: `cli::parse` hands everything after
    /// the verb to the verb, and this is where it is caught.
    #[test]
    fn following_a_verb_that_has_no_stream_is_refused_rather_than_passed_through() {
        let out = piped("rivet", &["explain", "network", "--follow"]);
        assert_ne!(out.code, 0);
        assert!(out.stderr.contains("--follow"), "{}", out.stderr);
    }

    // -- the stdout invariant --------------------------------------------------

    /// **The most expensive thing in this crate to get wrong.** Measured on
    /// hardware: a single greeting line on stdout made `scp` fail with
    /// `Received message too long 1717924466` — the greeting's own first four
    /// bytes read as an SFTP packet length, in an error naming neither the shell
    /// nor the greeting.
    ///
    /// Every refusal this binary can produce, walked, asserting stdout is
    /// **byte-empty**. Injecting a `println!` of any refusal fails this.
    #[test]
    fn a_refusal_writes_nothing_at_all_to_stdout() {
        let cases: Vec<(&str, Vec<&str>)> = vec![
            ("-rivet", vec![]),                                     // interactive login
            ("rivet", vec!["-c", "/usr/lib/sftp-server"]),          // sftp
            ("rivet", vec!["-c", "scp -t /tmp/"]),                  // scp -O
            ("rivet", vec!["-c", "rivet get $HOME"]),             // a metacharacter
            ("rivet", vec!["-c", "   "]),                           // empty command
            ("rivet", vec!["-c"]),                                  // -c with no string
            ("rivet", vec!["-c", "rivet get x", "a", "b"]),       // extra words
            ("rivet", vec!["list"]),                                // unimplemented
            ("rivet", vec!["explain"]),                             // implemented, no argument
            // `/etc/motd` until 2026-08-26, when ADR-035 §9B.1 made it a
            // generated view and `explain` started answering for it.
            ("rivet", vec!["explain", "/etc/fstab"]),               // implemented, unknown path
            ("rivet", vec!["apply", "trial-boot", "b"]),            // the plan, unconfirmed
            ("rivet", vec!["apply", "trial-boot", "a", "confirm"]), // an interlock
            ("rivet", vec!["apply", "config", "b"]),                // an unknown subject
            ("rivet", vec!["apply", "trial-boot", "c"]),            // not a slot
            ("rivet", vec!["apply", "trial-boot", "b", "yes"]),     // not the confirmation
            ("rivet", vec!["apply", "power-off"]),                  // a stop plan
            ("rivet", vec!["apply", "restart"]),                    // a stop plan
            ("rivet", vec!["apply", "trial-restart"]),              // a stop plan
            ("rivet", vec!["apply", "power-off", "confrim"]),       // a misspelt confirmation
            ("rivet", vec!["apply", "power-off", "b", "confirm"]),  // a stop takes no slot
            ("rivet", vec!["apply", "poweroff", "confirm"]),        // not a subject
            ("rivet", vec!["apply", "halt", "confirm"]),            // habit from another Linux
            ("rivet", vec!["apply", "shutdown", "confirm"]),        // ditto
            ("rivet", vec!["measure"]),                             // no subject
            ("rivet", vec!["measure", "hash"]),                     // no path
            ("rivet", vec!["measure", "throughput", "/dev/x"]),     // unknown subject
            (
                "rivet",
                vec!["measure", "hash", "/rivet-no-such-device"],
            ), // no such path
            ("rivet", vec!["frobnicate"]),                          // unknown verb
            ("rivet", vec!["--wat"]),                               // unknown option
            ("rivet", vec![]),                                      // nothing at all
            ("rivet", vec!["--output", "yaml", "get"]),             // bad output mode
        ];
        for (argv0, args) in cases {
            let streams = piped(argv0, &args);
            assert_ne!(streams.code, 0, "{argv0} {args:?} reported success");
            assert_eq!(
                streams.stdout, "",
                "{argv0} {args:?} wrote {:?} to stdout; measured to cost a failed scp",
                streams.stdout
            );
            assert!(
                !streams.stderr.is_empty(),
                "{argv0} {args:?} refused silently"
            );
        }
    }

    /// The other half: a refusal must be *loud*. Every one carries its code, so
    /// an agent can branch on it without reading English (`ADR-006` §4.1).
    #[test]
    fn every_refusal_carries_its_code_on_stderr() {
        for (args, code) in [
            // `list`, not `get`: `get` grew an implementation on 2026-08-24
            // (ADR-022 §9's journal query), and a test whose subject is "a verb
            // with no operation behind it" has to name one that still is.
            (vec!["list"], Code::UNIMPLEMENTED),
            (vec!["frobnicate"], Code::UNKNOWN_VERB),
            (vec!["-c", "/usr/lib/sftp-server"], Code::NotAShell),
            (vec!["--wat"], Code::ARGUMENTS),
            (
                vec!["explain", "/etc/fstab"],
                Code::Operation(ErrorCode::UnknownSubject),
            ),
            (
                vec!["apply", "trial-boot", "b"],
                Code::Operation(ErrorCode::ConfirmationRequired),
            ),
            (
                vec!["apply", "trial-boot", "a", "confirm"],
                Code::Operation(ErrorCode::Unsafe),
            ),
            (
                vec!["apply", "config", "b"],
                Code::Operation(ErrorCode::UnknownSubject),
            ),
            (
                vec!["apply", "power-off"],
                Code::Operation(ErrorCode::ConfirmationRequired),
            ),
            (
                vec!["apply", "trial-restart"],
                Code::Operation(ErrorCode::ConfirmationRequired),
            ),
            // `halt` is what a person types from habit on any other Linux, and
            // it must be an unknown SUBJECT rather than an unknown verb: the
            // verb was fine and this build has three ways to stop a machine.
            (
                vec!["apply", "halt", "confirm"],
                Code::Operation(ErrorCode::UnknownSubject),
            ),
            (
                vec!["apply", "power-off", "b", "confirm"],
                Code::Operation(ErrorCode::Arguments),
            ),
        ] {
            let streams = piped("rivet", &args);
            assert!(
                streams.stderr.contains(code.as_str()),
                "{args:?} did not report {}: {}",
                code.as_str(),
                streams.stderr
            );
            assert_eq!(streams.code, code.exit_code(), "{args:?}");
        }
        let login = piped("-rivet", &[]);
        assert!(login.stderr.contains(Code::NoInteractiveShell.as_str()));
        assert_eq!(login.code, 126);
    }

    // -- the login-shell contract ---------------------------------------------

    /// A bare remote login. The refusal must name `nushell`, because the
    /// operator's next question is what the shell is *supposed* to be.
    #[test]
    fn an_interactive_login_is_refused_loudly_and_says_what_the_shell_should_be() {
        let streams = piped("-rivet", &[]);
        assert_eq!(streams.code, 126);
        assert!(streams.stderr.contains("nushell"), "{}", streams.stderr);
        assert!(
            streams.stderr.contains("cargo rivet run"),
            "the refusal must say what to do instead, now that a login is not it: {}",
            streams.stderr
        );
        assert!(
            streams.stderr.contains("E_NO_INTERACTIVE_SHELL"),
            "{}",
            streams.stderr
        );
    }

    /// A remote `rivet --version` — the one shape that works end to end, and
    /// the reason listing this binary in `/etc/shells` buys anything at all.
    #[test]
    fn a_remote_rivet_version_produces_a_value_and_exits_zero() {
        let streams = piped("rivet", &["-c", "rivet --version"]);
        assert_eq!(streams.code, 0, "stderr: {}", streams.stderr);
        assert_eq!(streams.stderr, "");
        assert!(
            streams.stdout.contains("\"name\":\"rivet\""),
            "{}",
            streams.stdout
        );
        assert!(streams.stdout.contains(INSTALL_PATH), "{}", streams.stdout);
    }

    /// The install path is accepted too, since a `-c` string may spell it out.
    #[test]
    fn the_install_path_works_as_the_command_name_in_a_dash_c_string() {
        let streams = piped("rivet", &["-c", "/system/bin/rivet --version"]);
        assert_eq!(streams.code, 0, "stderr: {}", streams.stderr);
    }

    /// `argv[0]` carrying the login dash must not change what a command does —
    /// only what a *bare* invocation means.
    #[test]
    fn the_login_dash_does_not_change_the_answer_when_there_is_a_command() {
        assert_eq!(
            run_on_board("-rivet", &argv(&["--version"]), false),
            run_on_board("rivet", &argv(&["--version"]), false)
        );
    }

    // -- structured output -----------------------------------------------------

    /// `ADR-006` §4's default, driven through the whole program rather than
    /// through `Output::resolve` alone.
    #[test]
    fn a_pipe_gets_json_and_a_terminal_gets_the_human_rendering() {
        let piped = run_on_board("rivet", &argv(&["--version"]), false);
        assert!(piped.stdout.starts_with('{'), "{}", piped.stdout);

        let terminal = run_on_board("rivet", &argv(&["--version"]), true);
        assert!(!terminal.stdout.starts_with('{'), "{}", terminal.stdout);
        assert!(
            terminal.stdout.contains("name: rivet"),
            "{}",
            terminal.stdout
        );
    }

    /// **The property `ADR-006` §4 turns into a defect if it fails**: the two
    /// renderings cannot disagree, because neither has a source of its own.
    #[test]
    fn the_human_rendering_says_nothing_the_machine_form_does_not() {
        let json = run_on_board("rivet", &argv(&["--output", "json", "--version"]), true).stdout;
        let text = run_on_board("rivet", &argv(&["--output", "text", "--version"]), false).stdout;
        for needle in ["rivet", env!("CARGO_PKG_VERSION"), INSTALL_PATH] {
            assert!(json.contains(needle), "json dropped {needle}: {json}");
            assert!(text.contains(needle), "text dropped {needle}: {text}");
        }
    }

    /// A refusal renders through the same two readers, so an agent driving
    /// a remote `rivet get x` gets a parseable error rather than prose.
    #[test]
    fn a_refusal_down_a_pipe_is_json() {
        // `list` rather than `get` — see `every_refusal_carries_its_code_on_stderr`.
        let streams = piped("rivet", &["list"]);
        assert!(streams.stderr.starts_with('{'), "{}", streams.stderr);
        assert!(
            streams.stderr.contains(r#""error":"E_UNIMPLEMENTED""#),
            "{}",
            streams.stderr
        );
        assert!(
            streams.stderr.contains(r#""subject":"list""#),
            "{}",
            streams.stderr
        );
    }

    // -- the operations that are here, and the ones that are not ---------------

    /// **`ADR-006` §3.2's conformance property, at the adapter.**
    ///
    /// > The operation catalogue of section 7 is generated from the core's
    /// > types. That makes co-equality a **conformance property** rather than an
    /// > aspiration: an adapter can be checked against the catalogue, and one
    /// > that cannot reach a catalogued operation is reported as incomplete.
    ///
    /// So this walks the catalogue — not a list written here — and drives each
    /// entry through `argv`. An operation the core catalogues and this CLI
    /// cannot reach is "a defect in that interface, not a feature of it"
    /// (§3.1), and it fails here.
    ///
    /// **What "reach" means was narrowed on 2026-08-23, and the narrowing is
    /// deliberate rather than a weakening.** It used to assert exit 0 for every
    /// entry, which was available while every catalogued operation was read-only
    /// and would answer anything. It is not available now: `apply` is
    /// mutating, and an invocation that produced a *value* for every catalogue
    /// entry would mean this test armed a trial boot on the build machine. So
    /// what is asserted is the property §3.2 is actually about — the request
    /// reached the operation — and the two codes that mean it did not are the
    /// failure. `rivet-core`'s half of this test made the same distinction
    /// first, and for the same reason.
    #[test]
    fn every_catalogued_operation_is_reachable_from_this_adapter() {
        for op in operation::CATALOGUE {
            let mut args = vec![op.verb.as_str()];
            // A value each declared input accepts *syntactically*. `/etc/passwd`
            // is a path `explain` answers about and a subject `apply` refuses by
            // name; either way the operation saw it, which is what is being
            // tested.
            args.extend(op.inputs.iter().map(|_| "/etc/passwd"));
            let streams = piped("rivet", &args);
            let answered = streams.stdout.clone() + &streams.stderr;
            for unreachable in ["E_UNIMPLEMENTED", "E_UNKNOWN_VERB", "E_ARGUMENTS"] {
                assert!(
                    !answered.contains(unreachable),
                    "{} is catalogued and this CLI could not reach it: {answered}",
                    op.verb
                );
            }
            assert!(!answered.is_empty(), "{} answered nothing at all", op.verb);
        }
    }

    /// The other half, and the one the narrowing above gives up: **every
    /// catalogued operation that is read-only still produces a value.**
    ///
    /// It walks the catalogue and skips the **one** verb this build knows to be
    /// mutating, which is named here because there is nothing in the catalogue
    /// to derive it from: `ADR-006` §6.5's framework is what would make
    /// "mutating" a property of the type. When it lands, the skip becomes a
    /// filter over a field and the test above can assert exit 0 for the rest
    /// again.
    ///
    /// **The arguments are named per verb too, and for the same kind of
    /// reason.** They used to be `/etc/passwd` repeated, which worked while the
    /// only read-only operation took a path. `get` takes a *subject* from a
    /// closed set (`ADR-006` §6.4's nouns are values), so a catalogue entry
    /// declares what an input is **called** and not what it may be — and the
    /// day it declares that, this table becomes a derivation.
    #[test]
    fn every_read_only_operation_in_the_catalogue_produces_a_value() {
        let mut seen = 0usize;
        for op in operation::CATALOGUE {
            if op.verb == Verb::Apply {
                continue;
            }
            let args: Vec<&str> = match op.verb {
                Verb::Explain => vec!["explain", "/etc/passwd"],
                // No `boot` or `limit`: both are optional, and defaulting is
                // what an operator types.
                Verb::Get => vec!["get", operation::LOG],
                other => panic!(
                    "{other} was catalogued and this test does not know what a valid \
                     invocation of it looks like. Add one — a read-only operation nothing \
                     ever runs is not covered by anything here."
                ),
            };
            let streams = piped("rivet", &args);
            assert_eq!(streams.code, 0, "{}: {}", op.verb, streams.stderr);
            assert!(!streams.stdout.is_empty(), "{} produced no value", op.verb);
            seen += 1;
        }
        assert!(
            seen > 0,
            "every catalogued operation was skipped as mutating"
        );
    }

    /// **`rivet get log` on a machine that is not a Rivet answers**, rather
    /// than refusing — which is what makes this operation testable at all
    /// without a board.
    ///
    /// The development host has no `/state/log`, so the honest answer is an
    /// empty journal that names where it looked. A refusal here would have been
    /// a stub pretending the question could not be asked (`AGENTS.md` §3), and
    /// `crate::journal`'s module documentation argues it.
    #[test]
    fn get_log_on_a_host_with_no_journal_reports_an_empty_one_and_names_the_place() {
        let streams = piped("rivet", &["get", operation::LOG]);
        assert_eq!(streams.code, 0, "{}", streams.stderr);
        assert!(
            streams.stdout.contains(r#""subject":"log""#),
            "{}",
            streams.stdout
        );
        // Not `"returned":0` — this reads the real filesystem, and a host that
        // happened to have a journal would fail a test whose actual claim is
        // "it answers rather than refusing". `unreadable` is asserted because
        // it must be 0 in either case: whatever was found, it parsed.
        assert!(
            streams.stdout.contains(r#""unreadable":0"#),
            "{}",
            streams.stdout
        );
        assert!(
            streams.stdout.contains(rivet_core::state::LOG_DIR),
            "the answer does not say where it looked: {}",
            streams.stdout
        );
    }

    /// **`rivet get storage`, driven the way an operator drives it** — as a
    /// remote command, which is how `ADR-006` §5 rules the administration
    /// interface is reached by.
    ///
    /// This reaches the **real** `/proc/self/mounts` and the real `statvfs(3)`,
    /// so what it settles depends on where the suite runs, and both directions
    /// are asserted rather than one being skipped:
    ///
    /// * **On Linux** — WSL, or the board — it must produce a value naming the
    ///   table it read and carrying at least one filesystem.
    /// * **On Windows** it must **refuse** with `E_MACHINE_REFUSED`, because a
    ///   Windows host has filesystems this build cannot see and reporting none
    ///   would be false (`AGENTS.md` §3). A refusal there is the correct
    ///   behaviour, not a skipped test.
    ///
    /// **What neither settles** (`AGENTS.md` §6): the numbers, on a card. The
    /// host running this is a development machine; `ADR-026` §4 puts `ADR-018`'s
    /// partition layout on real media in the board's column.
    #[test]
    fn get_storage_as_a_remote_command_answers_on_linux_or_refuses_where_it_cannot_measure() {
        let streams = piped("rivet", &["-c", "rivet get storage"]);
        if streams.code != 0 {
            assert!(
                streams.stderr.contains("E_MACHINE_REFUSED"),
                "storage failed for a reason other than being unable to measure: {}",
                streams.stderr
            );
            assert!(
                streams.stderr.contains("NOTHING WAS MEASURED"),
                "the refusal must say that no partial answer was produced: {}",
                streams.stderr
            );
            assert_eq!(streams.stdout, "", "nothing on stdout");
            return;
        }

        assert_eq!(streams.stderr, "");
        for field in [
            r#""subject":"storage""#,
            r#""filesystems":["#,
            r#""mount-point":"#,
            r#""fstype":"#,
            r#""total-bytes":"#,
            r#""used-bytes":"#,
            r#""available-bytes":"#,
            r#""count":"#,
            r#""unreadable":"#,
            r#""mount-table":"#,
        ] {
            assert!(
                streams.stdout.contains(field),
                "{field} missing: {}",
                streams.stdout
            );
        }
        // It names where it looked, so an empty answer would be diagnosable.
        assert!(
            streams.stdout.contains(rivet_adapter_system::MOUNT_TABLE),
            "the answer does not say where it looked: {}",
            streams.stdout
        );
        // And a running Linux has a root filesystem, so the answer is not empty.
        assert!(
            !streams.stdout.contains(r#""count":0"#),
            "a running Linux reported no filesystems: {}",
            streams.stdout
        );
    }

    /// A `SystemMedium` whose `statvfs` refuses for exactly the paths named,
    /// so a test can drive `rivet_adapter_system::read` — the real adapter
    /// conversion, not a hand-built `StorageRead` — over a mount that
    /// genuinely could not be measured.
    struct RefusingMedium {
        table: String,
        refuses: Vec<String>,
    }

    impl rivet_adapter_system::SystemMedium for RefusingMedium {
        fn read_file(&mut self, path: &str) -> Result<String, rivet_adapter_system::Refused> {
            assert_eq!(path, rivet_adapter_system::MOUNT_TABLE, "this medium reads one file");
            Ok(self.table.clone())
        }

        fn statvfs(
            &mut self,
            path: &str,
        ) -> Result<rivet_adapter_system::Usage, rivet_adapter_system::Refused> {
            if self.refuses.iter().any(|p| p == path) {
                return Err(rivet_adapter_system::Refused::new(5, "Input/output error"));
            }
            Ok(rivet_adapter_system::Usage {
                block_bytes: 4096,
                blocks: 1000,
                blocks_free: 500,
                blocks_available: 500,
                inodes: 0,
                inodes_free: 0,
                read_only: false,
            })
        }
    }

    /// **The shadowing fix, proven through the real adapter rather than a
    /// hand-built `StorageRead`.** `rivet_adapter_system::read` is handed a
    /// medium whose `statvfs` genuinely refuses for `/state` — exactly what a
    /// card with a failing mount produces — and `get storage` must still
    /// refuse to let `/` claim `/state`'s locations as its own.
    #[test]
    fn an_adapter_reading_with_a_genuinely_refusing_statvfs_still_shadows_correctly() {
        let mut medium = RefusingMedium {
            table: "/dev/mmcblk0p3 / erofs ro,relatime 0 0\n\
                     /dev/mmcblk0p5 /state ext4 rw,noatime 0 0\n"
                .to_string(),
            refuses: vec!["/state".to_string()],
        };
        let read = rivet_adapter_system::read(&mut medium).expect("a readable table");
        assert_eq!(read.filesystems.len(), 1, "{read:?}");
        assert_eq!(read.unreadable.len(), 1, "{read:?}");
        assert_eq!(read.unreadable[0].mount_point, "/state");

        let mut board = FakeBoard::new();
        board.filesystems_override = Some(read);
        let streams = run("rivet", &argv(&["get", "storage"]), false, &mut board);

        assert_eq!(streams.stderr, "", "{}", streams.stderr);
        assert!(streams.stdout.contains(r#""unreadable":1"#), "{}", streams.stdout);
        assert!(
            streams.stdout.contains(r#""holds":["/system"]"#),
            "an unreadable /state must not let / claim /state, /state/config or /state/log: {}",
            streams.stdout
        );
    }

    /// The human rendering says what the machine form says — `ADR-006` §4, over
    /// a result whose interesting parts are a nested list of records.
    #[test]
    fn the_human_rendering_of_storage_says_what_the_json_says() {
        let text = run_on_board(
            "rivet",
            &argv(&["--output", "text", "get", "storage"]),
            true,
        );
        if text.code != 0 {
            assert!(text.stderr.contains("E_MACHINE_REFUSED"), "{}", text.stderr);
            return;
        }
        for needle in [
            "subject: storage",
            "filesystems:",
            "mount-point:",
            "total-bytes:",
            "mount-table:",
        ] {
            assert!(
                text.stdout.contains(needle),
                "{needle} missing: {}",
                text.stdout
            );
        }
        // Bytes are integers in BOTH readings. A human rendering that turned
        // them into `4.9 GiB` would be saying something the machine form does
        // not, which ADR-006 §4 forbids — and `crate::value::scalar` is what
        // stops it.
        assert!(
            !text.stdout.contains("GiB") && !text.stdout.contains("MiB"),
            "the text rendering invented a unit: {}",
            text.stdout
        );
    }

    /// **`rivet get storage /state` is refused, not silently ignored.** It is
    /// what a person types from `df` habit, and an operator shown the whole
    /// machine would reasonably read it as the answer for `/state`.
    #[test]
    fn get_storage_given_a_path_is_refused_and_told_how_to_select_instead() {
        let streams = piped("rivet", &["get", "storage", "/state"]);
        assert_eq!(streams.code, 2, "{}", streams.stderr);
        assert_eq!(streams.stdout, "", "nothing on stdout");
        assert!(streams.stderr.contains("E_ARGUMENTS"), "{}", streams.stderr);
        assert!(
            streams.stderr.contains("where mount-point"),
            "the refusal must say what to type instead: {}",
            streams.stderr
        );
    }

    /// And the argument surface, through the whole binary: a subject this build
    /// does not read is refused by name rather than answered with an empty log.
    ///
    /// **The word was `network` until 2026-08-29**, when that became a real
    /// subject — `ip` replaces it as the plausible-but-unread one, which is the
    /// property this test needs.
    #[test]
    fn get_refuses_a_subject_it_does_not_read() {
        let streams = piped("rivet", &["get", "ip"]);
        assert_eq!(streams.code, 1);
        assert_eq!(streams.stdout, "", "nothing on stdout");
        assert!(
            streams.stderr.contains("E_UNKNOWN_SUBJECT"),
            "{}",
            streams.stderr
        );
    }

    // -- arming a trial boot, end to end ---------------------------------------

    /// **`ADR-012` §7.1 through the whole binary**: the words an operator types
    /// as a remote command, and a machine that is not touched without the confirmation.
    #[test]
    fn arming_remotely_shows_the_plan_first_and_arms_only_when_confirmed() {
        let mut board = FakeBoard::new();

        let plan = run(
            "rivet",
            &argv(&["-c", "rivet apply trial-boot b"]),
            false,
            &mut board,
        );
        assert_eq!(plan.code, 1, "a plan is a refusal, not a result");
        assert_eq!(plan.stdout, "", "nothing on stdout");
        assert!(
            plan.stderr.contains("E_CONFIRMATION_REQUIRED"),
            "{}",
            plan.stderr
        );
        assert!(board.armed.is_empty(), "the plan armed something");

        let armed = run(
            "rivet",
            &argv(&["-c", "rivet apply trial-boot b confirm"]),
            false,
            &mut board,
        );
        assert_eq!(armed.code, 0, "stderr: {}", armed.stderr);
        assert_eq!(armed.stderr, "");
        assert_eq!(board.armed, vec![Slot::B]);
        assert!(armed.stdout.contains(r#""armed":true"#), "{}", armed.stdout);
        assert!(
            armed.stdout.contains(r#""rebooted":false"#),
            "arming must never report a restart: {}",
            armed.stdout
        );
    }

    /// The interlock's refusal reaches the operator with its own code, so an
    /// agent stops rather than retrying — and with the message that says what to
    /// do instead.
    #[test]
    fn an_interlock_refusal_reaches_the_operator_with_its_own_code() {
        let streams = piped("rivet", &["apply", "trial-boot", "a", "confirm"]);
        assert_eq!(streams.code, 1);
        assert_eq!(streams.stdout, "");
        assert!(streams.stderr.contains("E_UNSAFE"), "{}", streams.stderr);
        assert!(
            streams.stderr.contains("running slot"),
            "the interlock's own words must survive: {}",
            streams.stderr
        );
    }

    /// The human rendering of an arming says what the machine form says — the
    /// property `ADR-006` §4 turns into a defect if it fails.
    #[test]
    fn the_human_rendering_of_an_arming_says_what_the_json_says() {
        let args = argv(&["--output", "text", "apply", "trial-boot", "b", "confirm"]);
        let text = run_on_board("rivet", &args, true).stdout;
        assert!(text.contains("armed: true"), "{text}");
        assert!(text.contains("rebooted: false"), "{text}");
        assert!(text.contains("reboot: one-shot"), "{text}");
        assert!(text.contains("next:"), "{text}");
    }

    // -- restarting one service, end to end -----------------------------------

    /// **`rivet apply service-restart <name>` reaches the operation, plans, and
    /// only the confirmed form asks.** `ADR-017` §6.3.2.
    ///
    /// What this crate owns is `argv` reaching the operation and a value coming
    /// back; the refusals live in the supervisor and are tested there and in
    /// `rivet-core`.
    #[test]
    fn restarting_one_service_plans_first_and_only_the_confirmed_form_asks() {
        let mut board = FakeBoard::new();
        let plan = run(
            "rivet",
            &argv(&["-c", "rivet apply service-restart photo-index"]),
            false,
            &mut board,
        );
        assert_eq!(plan.code, 1, "a plan is a refusal, not a result");
        assert_eq!(plan.stdout, "", "nothing on stdout");
        assert!(
            plan.stderr.contains("E_CONFIRMATION_REQUIRED"),
            "{}",
            plan.stderr
        );
        assert!(
            board.restarted.is_empty(),
            "the plan restarted something: {:?}",
            board.restarted
        );
        // The plan names the two remedies for the thing it will NOT do.
        assert!(plan.stderr.contains("apply edit"), "{}", plan.stderr);
        assert!(plan.stderr.contains("apply reload"), "{}", plan.stderr);

        let done = run(
            "rivet",
            &argv(&["-c", "rivet apply service-restart photo-index confirm"]),
            false,
            &mut board,
        );
        assert_eq!(done.code, 0, "stderr: {}", done.stderr);
        assert_eq!(board.restarted, vec!["photo-index".to_string()]);
        assert!(done.stdout.contains(r#""was":"started""#), "{}", done.stdout);
        assert!(done.stdout.contains(r#""doing":"stop""#), "{}", done.stdout);
    }

    /// **`rivet apply service-stop` and `service-start` reach the operation,
    /// plan first, and only the confirmed form asks — and neither stops the
    /// machine.** `ADR-017` §6.
    #[test]
    fn stopping_and_starting_one_application_plans_first_and_never_stops_the_machine() {
        let mut board = FakeBoard::new();
        for subject in ["service-stop", "service-start"] {
            let plan = run(
                "rivet",
                &argv(&["-c", &format!("rivet apply {subject} photo-index")]),
                false,
                &mut board,
            );
            assert_eq!(plan.code, 1, "a plan is a refusal, not a result");
            assert!(
                plan.stderr.contains("E_CONFIRMATION_REQUIRED"),
                "{}",
                plan.stderr
            );
            assert!(board.held.is_empty(), "the plan asked: {:?}", board.held);

            let done = run(
                "rivet",
                &argv(&["-c", &format!("rivet apply {subject} photo-index confirm")]),
                false,
                &mut board,
            );
            assert_eq!(done.code, 0, "stderr: {}", done.stderr);
            assert!(
                done.stdout.contains(&format!(r#""subject":"{subject}""#)),
                "{}",
                done.stdout
            );
            board.held.retain(|h| h != &format!("{subject} photo-index"));
            assert!(board.held.is_empty(), "{:?}", board.held);
        }
        // The stop plan says the application's directory is untouched.
        let plan = run(
            "rivet",
            &argv(&["-c", "rivet apply service-stop photo-index"]),
            false,
            &mut board,
        );
        assert!(plan.stderr.contains("/state/app/photo-index"), "{}", plan.stderr);
        assert!(board.stopped.is_empty(), "a service stop stopped the machine");
        assert!(board.restarted.is_empty(), "a service stop restarted something");
    }

    /// **The two `restart`s are different commands and the machine one still
    /// restarts the machine.** The failure this guards is an operator's fingers
    /// rebooting a board when they meant to bounce a daemon.
    #[test]
    fn apply_restart_still_restarts_the_machine_and_service_restart_does_not() {
        let mut board = FakeBoard::new();
        let machine = run(
            "rivet",
            &argv(&["-c", "rivet apply restart confirm"]),
            false,
            &mut board,
        );
        assert_eq!(machine.code, 0, "stderr: {}", machine.stderr);
        assert_eq!(board.stopped, vec![Stop::Restart]);
        assert!(
            board.restarted.is_empty(),
            "a machine restart touched a service"
        );

        let mut board = FakeBoard::new();
        let service = run(
            "rivet",
            &argv(&["-c", "rivet apply service-restart photo-index confirm"]),
            false,
            &mut board,
        );
        assert_eq!(service.code, 0, "stderr: {}", service.stderr);
        assert!(
            board.stopped.is_empty(),
            "a service restart stopped the machine: {:?}",
            board.stopped
        );
    }

    // -- installing, end to end ------------------------------------------------

    /// ~~**`rivet apply install` refuses, everywhere, and says why.**~~
    ///
    /// **Rewritten 2026-08-24 evening, when the channel was ruled.** What this
    /// asserted until then was the honest state of the system — how a payload
    /// reached `rivet-deploy` was not ruled, so `apply install` refused with
    /// `E_UNSAFE` on a board and a workstation alike. The maintainer's 2026-08-24
    /// install-request ruling took option A, and the command now **delivers**: it writes a request onto
    /// `/state` and returns before a byte of the install is written.
    ///
    /// What this asserts is what this crate owns: `argv` reaches the operation,
    /// the request lands in the form the supervisor's own parser reads, and the
    /// answer says *delivered* rather than *installed*.
    #[test]
    fn a_confirmed_install_delivers_a_request_and_says_it_is_not_an_install() {
        let mut board = FakeBoard::new();
        let out = run(
            "rivet",
            &argv(&["-c", &format!("rivet apply install {A_DIGEST} confirm")]),
            false,
            &mut board,
        );
        assert_eq!(out.code, 0, "{}", out.stderr);
        assert!(out.stdout.contains(r#""requested":true"#), "{}", out.stdout);
        assert!(
            out.stdout.contains(r#""installed":false"#),
            "an answer that said the machine had installed something would be a lie: {}",
            out.stdout
        );

        // And the request is on the card, readable by the parser the supervisor
        // uses — the two halves of the channel meeting through the whole binary.
        let Some(request) = board.installs.store().request() else {
            unreachable!("nothing was written to the card")
        };
        assert_eq!(
            rivet_core::deploy::parse_request(request).map(|r| r.digest),
            Ok(a_digest())
        );
    }

    /// **The plan path delivers nothing**, and says so in the sentence an
    /// operator reads before deciding whether to confirm.
    #[test]
    fn the_plan_path_leaves_the_card_untouched() {
        let mut board = FakeBoard::new();
        let out = run(
            "rivet",
            &argv(&["-c", &format!("rivet apply install {A_DIGEST}")]),
            false,
            &mut board,
        );
        assert_ne!(out.code, 0, "an unconfirmed install succeeded");
        assert!(
            out.stderr.contains("E_CONFIRMATION_REQUIRED"),
            "{}",
            out.stderr
        );
        assert!(
            out.stderr.contains("NOTHING HAS BEEN HANDED OVER"),
            "{}",
            out.stderr
        );
        assert!(
            board.installs.store().request().is_none(),
            "the plan path wrote a request"
        );
    }

    /// **A second request while one is waiting is refused, not silently
    /// swapped.** The operator who asked for the first was told it was
    /// delivered.
    #[test]
    fn a_second_install_while_one_waits_is_refused_and_names_the_one_that_is_waiting() {
        let mut board = FakeBoard::new();
        let first = run(
            "rivet",
            &argv(&["-c", &format!("rivet apply install {A_DIGEST} confirm")]),
            false,
            &mut board,
        );
        assert_eq!(first.code, 0, "{}", first.stderr);

        let second = run(
            "rivet",
            &argv(&["-c", &format!("rivet apply install {ANOTHER_DIGEST} confirm")]),
            false,
            &mut board,
        );
        assert_ne!(second.code, 0, "the second request was accepted");
        assert!(
            second.stderr.contains(A_DIGEST),
            "the refusal did not name what is waiting: {}",
            second.stderr
        );
        // **The first request, byte for byte, principal and all.** Rendered
        // rather than spelled out: this test is about the second command not
        // touching it, and a literal here would fail for the unrelated reason
        // that this host's own environment differs from the one that wrote it.
        assert_eq!(
            board.installs.store().request().map(String::as_str),
            Some(
                rivet_core::deploy::render_request(
                    &a_digest(),
                    &crate::deploy::this_process()
                )
                .as_str()
            ),
            "the waiting request was replaced"
        );
    }

    /// A board holding one build of an addon product and one application that
    /// joins it by class — `apply uninstall-library`'s ordinary case.
    fn holding_an_addon() -> (FakeBoard, rivet_core::package::library::Reference) {
        let pack = rivet_core::package::library::Reference::compose(
            "aura-background-pack",
            "0.2.0",
            &[0xaa; 32],
        )
        .unwrap_or_else(|e| unreachable!("a fixture reference: {e:?}"));
        let mut board = FakeBoard::new();
        board.installs = crate::deploy::Requests::new(
            crate::deploy::Memory::rivet()
                .with_library(&pack, Some("aura-background-pack"))
                .with_application("aura-renderer", &[], &["aura-background-pack"]),
            crate::deploy::this_process(),
        );
        (board, pack)
    }

    /// **`apply uninstall-library` end to end through the binary**: `argv`
    /// reaches the operation, the request lands on the card in the bytes the
    /// supervisor's own reader parses, and the answer says *delivered* rather
    /// than *removed*.
    #[test]
    fn a_confirmed_library_uninstall_delivers_a_request_the_supervisor_reads() {
        let (mut board, pack) = holding_an_addon();
        let out = run(
            "rivet",
            &argv(&["-c", "rivet apply uninstall-library aura-background-pack confirm"]),
            false,
            &mut board,
        );
        assert_eq!(out.code, 0, "{}", out.stderr);
        assert!(out.stdout.contains(r#""requested":true"#), "{}", out.stdout);
        assert!(out.stdout.contains(r#""removed":false"#), "{}", out.stdout);
        assert!(out.stdout.contains(pack.as_str()), "{}", out.stdout);
        assert!(out.stdout.contains("aura-renderer accepts"), "{}", out.stdout);

        let Some(request) = board.installs.store().library_request() else {
            unreachable!("nothing was written to the card")
        };
        assert_eq!(
            rivet_core::uninstall_library::parse_request(request).map(|r| r.selector),
            Ok(rivet_core::uninstall_library::Selector::Product(
                "aura-background-pack".to_string()
            ))
        );
    }

    /// **The plan path writes nothing**, and the plan names the build it would
    /// remove and the application that loses it.
    #[test]
    fn a_library_uninstall_plan_names_the_build_and_writes_nothing() {
        let (mut board, pack) = holding_an_addon();
        let out = run(
            "rivet",
            &argv(&["-c", "rivet apply uninstall-library aura-background-pack"]),
            false,
            &mut board,
        );
        assert_ne!(out.code, 0, "an unconfirmed removal succeeded");
        assert!(out.stderr.contains("E_CONFIRMATION_REQUIRED"), "{}", out.stderr);
        assert!(out.stderr.contains(pack.as_str()), "{}", out.stderr);
        assert!(out.stderr.contains("`aura-renderer`"), "{}", out.stderr);
        assert!(board.installs.store().library_request().is_none());
    }

    /// **The argument refusals happen before the channel is reached**, so an
    /// operator who typed the command wrongly is told *that* rather than being
    /// shown the unruled-channel sentence and left to think the two are
    /// connected.
    #[test]
    fn an_install_that_names_nothing_usable_is_an_argument_error_and_not_the_channel() {
        for (command, expected) in [
            ("rivet apply install", "E_ARGUMENTS"),
            ("rivet apply install confirm", "E_ARGUMENTS"),
            ("rivet apply install x.rvt confirm", "E_ARGUMENTS"),
        ] {
            let mut board = FakeBoard::new();
            let out = run("rivet", &argv(&["-c", command]), false, &mut board);
            assert_eq!(out.code, 2, "{command}: {}", out.stderr);
            assert!(out.stderr.contains(expected), "{command}: {}", out.stderr);
            assert!(
                !out.stderr.contains("NOTHING HAS BEEN DELIVERED"),
                "{command} reached the channel before checking its arguments: {}",
                out.stderr
            );
        }
    }

    /// **`rivet get install` answers on a machine with no install**, which is
    /// every machine this suite runs on. An empty answer that names where it
    /// looked is a true statement; a refusal would be a stub pretending the
    /// question could not be asked (`AGENTS.md` §3).
    #[test]
    fn asking_about_an_install_answers_even_when_there_has_never_been_one() {
        let mut board = FakeBoard::new();
        let out = run(
            "rivet",
            &argv(&["-c", "rivet get install"]),
            false,
            &mut board,
        );
        assert_eq!(out.code, 0, "{}", out.stderr);
        assert_eq!(out.stderr, "");
        assert!(
            out.stdout.contains(r#""subject":"install""#),
            "{}",
            out.stdout
        );
        // On a development host there is no `/state/log`, so the honest answer
        // is zero installs and the place that was looked in.
        assert!(out.stdout.contains(r#""installs":0"#), "{}", out.stdout);
        assert!(out.stdout.contains(r#""source":"#), "{}", out.stdout);
        assert!(
            out.stdout.contains("HOLDS NO INSTALL"),
            "an empty answer must say what it does and does not mean: {}",
            out.stdout
        );
    }

    /// The human rendering says what the machine form says — `ADR-006` §4: if
    /// the two can disagree, the rendering is reading from the wrong source.
    #[test]
    fn the_human_rendering_of_an_install_report_says_what_the_json_says() {
        let args = argv(&["--output", "text", "get", "install"]);
        let text = run_on_board("rivet", &args, true).stdout;
        assert!(text.contains("subject: install"), "{text}");
        assert!(text.contains("installs: 0"), "{text}");
        assert!(text.contains("next:"), "{text}");
    }

    // -- editing a file, end to end --------------------------------------------

    /// **The maintainer's own example, driven the way an operator reaches it
    /// today**: as `argv`, which is what `nushell` execs once it has done the
    /// quoting — `ADR-005` §5.1 makes `nushell` the interactive login shell.
    ///
    /// **It is deliberately NOT driven through `-c`**, and that is a finding
    /// rather than a convenience.
    /// `a_dash_c_command_carrying_quoted_text_is_refused_and_that_is_the_gap`
    /// below pins why; `PLAN.md` carries the question.
    ///
    /// The plan first, then the confirmation, and the file changes exactly once.
    /// It is a `FakeBoard` over a filesystem in a map — `AGENTS.md` §6 applies:
    /// **no file on any machine has been changed by this test.**
    #[test]
    fn editing_a_file_shows_the_plan_first_and_writes_only_when_confirmed() {
        let mut board = FakeBoard::new();
        let command = [
            "apply",
            "edit",
            FakeBoard::EDITABLE,
            "--expect",
            "$env.config.show_banner = true",
            "--replace",
            "$env.config.show_banner = false",
        ];

        let plan = run("rivet", &argv(&command), false, &mut board);
        assert_eq!(plan.code, 1, "{}", plan.stderr);
        assert_eq!(plan.stdout, "", "a plan must write nothing to stdout");
        assert!(
            plan.stderr.contains("E_CONFIRMATION_REQUIRED"),
            "{}",
            plan.stderr
        );
        assert!(plan.stderr.contains("LINE 2"), "{}", plan.stderr);
        assert_eq!(
            board.edits.store().text(FakeBoard::EDITABLE).as_deref(),
            Some("# nushell\n$env.config.show_banner = true\n"),
            "the plan changed the file"
        );

        let mut confirmed: Vec<&str> = command.to_vec();
        confirmed.push("confirm");
        let done = run("rivet", &argv(&confirmed), false, &mut board);
        assert_eq!(done.code, 0, "{}", done.stderr);
        assert_eq!(done.stderr, "");
        assert!(done.stdout.contains(r#""subject":"edit""#), "{}", done.stdout);
        assert!(
            done.stdout.contains(r#""change":"replace""#),
            "{}",
            done.stdout
        );
        assert!(done.stdout.contains(r#""line":2"#), "{}", done.stdout);
        assert!(
            done.stdout.contains(r#""journalled":true"#),
            "{}",
            done.stdout
        );
        assert_eq!(
            board.edits.store().text(FakeBoard::EDITABLE).as_deref(),
            Some("# nushell\n$env.config.show_banner = false\n")
        );

        // ...and the machine's own journal holds a record of it, which is the
        // half `ADR-012` §4 asks for and the nushell pipeline could never have.
        let lines = board.edits.store().journal();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains(r#""svc":"rivet""#), "{}", lines[0]);
        assert!(lines[0].contains(r#""change":"replace""#), "{}", lines[0]);
        assert!(lines[0].contains(r#""principal":"#), "{}", lines[0]);
    }

    /// ~~**THE GAP, pinned as a test rather than described in a comment.**~~
    ///
    /// **The gap is closed, and this test is the same command asserting the
    /// opposite** — `ADR-005` §5.4.1.3, ruled 2026-08-25. It is inverted rather
    /// than deleted because the sentence it used to pin is the reason the
    /// ruling exists, and the day somebody re-adds `'` to
    /// `SHELL_METACHARACTERS` this fails loudly instead of a stale comment
    /// going unnoticed.
    ///
    /// What it was: a remote `rivet apply edit … --expect (…) …` arrives
    /// as `-c <one string>`, and `crate::invocation::words`
    /// refused any string containing a character a shell would interpret — so
    /// **the maintainer's own worked example could not be typed as a
    /// non-interactive remote command at all**, and neither could any edit whose
    /// text had a space in it, which is nearly every interesting configuration
    /// line.
    ///
    /// What it is now: a single-quoted string has no escapes and no expansion,
    /// so splitting one is *provably* the answer a shell would give, and the
    /// command below **reaches the operation** and produces its plan.
    /// Everything else in that set is still refused —
    /// `every_other_shell_construct_is_still_refused` is the other half.
    #[test]
    fn a_dash_c_command_carrying_single_quoted_text_now_reaches_the_operation() {
        let mut board = FakeBoard::new();
        let out = run(
            "rivet",
            &argv(&[
                "-c",
                concat!(
                    "rivet apply edit /state/home/root/.config/nushell/config.nu ",
                    "--expect 'show_banner = true' --replace 'show_banner = false'"
                ),
            ]),
            false,
            &mut board,
        );
        assert_eq!(
            out.code, 1,
            "the quoted command must reach the operation and be answered with its plan: {}",
            out.stderr
        );
        assert!(
            out.stderr.contains("E_CONFIRMATION_REQUIRED"),
            "{}",
            out.stderr
        );
        assert!(
            out.stderr.contains("show_banner = true"),
            "the plan must quote the text the operator asked for, spaces and all: {}",
            out.stderr
        );
        assert_eq!(out.stdout, "", "a plan writes nothing to stdout");
        assert!(
            board.edits.store().journal().is_empty(),
            "a plan changes nothing"
        );

        // A `-c` command whose text carries no shell metacharacter and no space
        // DOES reach the operation, which is what says the refusal above is the
        // tokeniser's and not this operation's.
        let mut plain = FakeBoard::new();
        let reached = run(
            "rivet",
            &argv(&[
                "-c",
                "rivet apply edit /state/home/root/.config/nushell/config.nu --append banner",
            ]),
            false,
            &mut plain,
        );
        assert_eq!(reached.code, 1, "{}", reached.stderr);
        assert!(
            reached.stderr.contains("E_CONFIRMATION_REQUIRED"),
            "{}",
            reached.stderr
        );
    }

    /// **The refusal that is the whole point.** Text that occurs twice is
    /// ambiguous, and an ambiguous edit is refused rather than guessed —
    /// `E_UNSAFE`, which tells an agent to stop rather than to retry.
    #[test]
    fn an_ambiguous_edit_is_refused_and_leaves_the_file_alone() {
        let mut board = FakeBoard::new();
        let out = run(
            "rivet",
            &argv(&[
                "apply",
                "edit",
                FakeBoard::EDITABLE,
                "--expect",
                "sh",
                "--replace",
                "SH",
                "confirm",
            ]),
            false,
            &mut board,
        );
        assert_eq!(out.code, 1, "{}", out.stdout);
        assert_eq!(out.stdout, "");
        assert!(out.stderr.contains("E_UNSAFE"), "{}", out.stderr);
        assert!(out.stderr.contains("NEVER GUESSES"), "{}", out.stderr);
        assert_eq!(
            board.edits.store().text(FakeBoard::EDITABLE).as_deref(),
            Some("# nushell\n$env.config.show_banner = true\n")
        );
        assert!(board.edits.store().journal().is_empty());
    }

    /// **A view is refused with `explain`'s own answer**, so the two halves of
    /// the same question cannot disagree.
    #[test]
    fn editing_a_generated_path_is_refused_and_repeats_what_explain_says() {
        let mut board = FakeBoard::new();
        let out = run(
            "rivet",
            &argv(&["apply", "edit", "/etc/passwd", "--append", "x", "confirm"]),
            false,
            &mut board,
        );
        assert_eq!(out.code, 1, "{}", out.stdout);
        assert!(out.stderr.contains("E_UNSAFE"), "{}", out.stderr);
        assert!(out.stderr.contains("VIEW"), "{}", out.stderr);

        // The same machine, asked the read half of the question.
        let explained = run(
            "rivet",
            &argv(&["explain", "/etc/passwd"]),
            false,
            &mut board,
        );
        assert_eq!(explained.code, 0, "{}", explained.stderr);
        assert!(
            explained.stdout.contains("SYSTEM_IDENTITIES"),
            "{}",
            explained.stdout
        );
        assert!(
            out.stderr.contains("SYSTEM_IDENTITIES"),
            "the refusal does not repeat explain's answer: {}",
            out.stderr
        );
    }

    /// **`/state/deploy-request` is refused and names the mechanism**, which is
    /// the reserved path the ruling put first: writing it by hand forges a
    /// principal and bypasses `rivet apply install`.
    #[test]
    fn editing_the_install_request_is_refused_and_points_at_apply_install() {
        let mut board = FakeBoard::new();
        let out = run(
            "rivet",
            &argv(&[
                "apply",
                "edit",
                rivet_core::deploy::REQUEST_PATH,
                "--append",
                "x",
                "confirm",
            ]),
            false,
            &mut board,
        );
        assert_eq!(out.code, 1, "{}", out.stdout);
        assert!(out.stderr.contains("E_UNSAFE"), "{}", out.stderr);
        assert!(out.stderr.contains("USE THE MECHANISM"), "{}", out.stderr);
        assert!(
            out.stderr.contains("NOT A SECURITY BOUNDARY"),
            "{}",
            out.stderr
        );
        // ...and nothing reached the install channel.
        assert!(board.installs.store().request().is_none());
    }

    /// The human rendering says what the machine form says — `ADR-006` §4.
    #[test]
    fn the_human_rendering_of_an_edit_says_what_the_json_says() {
        let mut board = FakeBoard::new();
        let args = argv(&[
            "--output",
            "text",
            "apply",
            "edit",
            FakeBoard::EDITABLE,
            "--append",
            "$env.x = 1\n",
            "confirm",
        ]);
        let text = run("rivet", &args, true, &mut board).stdout;
        assert!(text.contains("subject: edit"), "{text}");
        assert!(text.contains("change: append"), "{text}");
        assert!(text.contains("created: false"), "{text}");
        assert!(text.contains("journalled: true"), "{text}");
        assert!(text.contains("principal:"), "{text}");
    }

    // -- stopping the machine, end to end --------------------------------------

    /// **The three commands that make the trial-boot experiment possible at
    /// all**, driven the way an operator drives them: one remote `rivet …`.
    ///
    /// Until 2026-08-23 nothing on a Rivet board could stop it. What this
    /// asserts is that `argv` reaches the operation and a value comes back — it
    /// is a `FakeBoard`, and `AGENTS.md` §6 applies: **no board has been stopped
    /// by this and none of these tests signals anything.**
    #[test]
    fn each_stop_subject_shows_its_plan_first_and_asks_only_when_confirmed() {
        for stop in Stop::ALL {
            let mut board = FakeBoard::new();

            let plan = run(
                "rivet",
                &argv(&["-c", &format!("rivet apply {stop}")]),
                false,
                &mut board,
            );
            assert_eq!(plan.code, 1, "{stop}: a plan is a refusal, not a result");
            assert_eq!(plan.stdout, "", "nothing on stdout");
            assert!(
                plan.stderr.contains("E_CONFIRMATION_REQUIRED"),
                "{stop}: {}",
                plan.stderr
            );
            assert!(
                board.stopped.is_empty(),
                "{stop}: the plan stopped the machine"
            );
            assert_eq!(
                board.planned_stops,
                vec![*stop],
                "{stop}: the plan skipped the interlock"
            );

            let done = run(
                "rivet",
                &argv(&["-c", &format!("rivet apply {stop} confirm")]),
                false,
                &mut board,
            );
            assert_eq!(done.code, 0, "{stop}: {}", done.stderr);
            assert_eq!(done.stderr, "");
            assert_eq!(board.stopped, vec![*stop], "{stop}");
            assert!(
                done.stdout.contains(r#""requested":true"#),
                "{stop}: {}",
                done.stdout
            );
            assert!(
                done.stdout.contains(r#""rebooted":false"#),
                "{stop}: a stop must never report a machine that came back: {}",
                done.stdout
            );
            assert!(
                done.stdout.contains(&format!(r#""subject":"{stop}""#)),
                "{stop}: {}",
                done.stdout
            );
        }
    }

    /// **The two acts, in the order a trial boot needs them**, over the one
    /// transport there is. Arming does not stop the machine; stopping does not
    /// arm it. Getting this backwards is a board that comes back healthy having
    /// trialled nothing.
    #[test]
    fn arming_and_the_trial_restart_are_two_commands_and_neither_does_the_others_job() {
        let mut board = FakeBoard::new();

        let armed = run(
            "rivet",
            &argv(&["-c", "rivet apply trial-boot b confirm"]),
            false,
            &mut board,
        );
        assert_eq!(armed.code, 0, "{}", armed.stderr);
        assert_eq!(board.armed, vec![Slot::B]);
        assert!(board.stopped.is_empty(), "arming stopped the machine");

        let restarted = run(
            "rivet",
            &argv(&["-c", "rivet apply trial-restart confirm"]),
            false,
            &mut board,
        );
        assert_eq!(restarted.code, 0, "{}", restarted.stderr);
        assert_eq!(board.stopped, vec![Stop::TrialRestart]);
        assert_eq!(board.armed, vec![Slot::B], "the restart armed something");
    }

    /// A stop subject takes no slot, and `apply power-off b confirm` must not
    /// power the machine off.
    #[test]
    fn a_stop_subject_given_a_slot_is_refused_rather_than_having_it_ignored() {
        let mut board = FakeBoard::new();
        let streams = run(
            "rivet",
            &argv(&["apply", "power-off", "b", "confirm"]),
            false,
            &mut board,
        );
        assert_eq!(streams.code, 2, "{}", streams.stderr);
        assert!(streams.stderr.contains("E_ARGUMENTS"), "{}", streams.stderr);
        assert_eq!(streams.stdout, "");
        assert!(
            board.stopped.is_empty(),
            "a malformed command stopped the machine"
        );
    }

    /// The human rendering of a stop says what the machine form says, and it
    /// carries the sentence an operator most needs: that the machine has not
    /// stopped yet and the console is the authority.
    #[test]
    fn the_human_rendering_of_a_stop_says_what_the_json_says_and_names_the_console() {
        let args = argv(&["--output", "text", "apply", "trial-restart", "confirm"]);
        let text = run_on_board("rivet", &args, true).stdout;
        assert!(text.contains("subject: trial-restart"), "{text}");
        assert!(text.contains("requested: true"), "{text}");
        assert!(text.contains("rebooted: false"), "{text}");
        assert!(text.contains("means:"), "{text}");
        assert!(text.contains("console"), "{text}");
        assert!(text.contains("SIGUSR1"), "{text}");
    }

    /// The other direction. Every verb with no catalogue entry refuses, and the
    /// refusal is the catalogued `Unimplemented` — not a message that happens to
    /// say so.
    ///
    /// **This test is what makes each next implemented operation a deliberate
    /// act**: implementing `get` without cataloguing it leaves a test asserting
    /// that `get` does nothing.
    #[test]
    fn every_verb_with_no_operation_behind_it_refuses_and_none_of_them_pretends() {
        for verb in Verb::ALL {
            if operation::operation(*verb).is_some() {
                continue;
            }
            let streams = piped("rivet", &[verb.as_str()]);
            assert_eq!(
                streams.code,
                Code::UNIMPLEMENTED.exit_code(),
                "{verb} did not refuse: {streams:?}"
            );
            assert!(
                streams.stderr.contains("E_UNIMPLEMENTED"),
                "{verb}: {}",
                streams.stderr
            );
            assert_eq!(streams.stdout, "", "{verb} produced output");
        }
    }

    /// **`ADR-006` §8.1 said `"operations":[]` was "the true count and the one
    /// thing about it that cannot be wrong". It is no longer the true count.**
    ///
    /// Asserted against the core's catalogue rather than against a literal, so
    /// that this cannot become a second list of operations.
    #[test]
    fn the_published_catalogue_is_the_one_the_core_performs() {
        let json = run_on_board("rivet", &argv(&["--output", "json", "--version"]), false).stdout;
        assert!(
            !json.contains(r#""operations":[]"#),
            "the catalogue is empty: {json}"
        );
        for op in operation::CATALOGUE {
            assert!(
                json.contains(&format!(r#""verb":"{}""#, op.verb)),
                "{} is not published: {json}",
                op.verb
            );
            for input in op.inputs {
                assert!(
                    json.contains(input.name),
                    "{} input missing: {json}",
                    op.verb
                );
            }
            for output in op.outputs() {
                assert!(
                    json.contains(output.name),
                    "{} output missing: {json}",
                    op.verb
                );
            }
            for code in op.errors {
                assert!(
                    json.contains(code.as_str()),
                    "{} error missing: {json}",
                    op.verb
                );
            }
        }
        // And the human rendering carries the same catalogue, per ADR-006 §4.
        let text = run_on_board("rivet", &argv(&["--output", "text", "--version"]), false).stdout;
        assert!(text.contains("operations:"), "{text}");
        for op in operation::CATALOGUE {
            assert!(
                text.contains(op.verb.as_str()),
                "{} missing: {text}",
                op.verb
            );
        }
    }

    /// The error contract is *readable*, not only provocable. `ADR-006` §4.1
    /// makes the codes part of the interface; an agent that has to trigger each
    /// failure to learn its code cannot plan around them.
    ///
    /// Asserted against `Code::all()` rather than a literal list, because the
    /// point is that the field is generated from the definitions — a copy here
    /// would be the second authority `ADR-011` forbids, in miniature.
    #[test]
    fn version_publishes_every_error_code_from_the_definition_rather_than_a_copy() {
        let json = run_on_board("rivet", &argv(&["--output", "json", "--version"]), false).stdout;
        for code in Code::all() {
            assert!(
                json.contains(code.as_str()),
                "{} missing: {json}",
                code.as_str()
            );
        }
    }

    // -- explain, end to end ---------------------------------------------------

    /// `ADR-007` §3.6.1 item 3, driven the way an operator would drive it —
    /// including as a remote command, which is how `ADR-006` §5 rules the
    /// administration interface is reached by.
    #[test]
    fn explaining_a_generated_path_answers_remotely_and_exits_zero() {
        let streams = piped("rivet", &["-c", "rivet explain /etc/passwd"]);
        assert_eq!(streams.code, 0, "stderr: {}", streams.stderr);
        assert_eq!(streams.stderr, "");
        assert!(
            streams.stdout.contains(r#""kind":"view""#),
            "{}",
            streams.stdout
        );
        assert!(
            streams.stdout.contains(r#""writable":false"#),
            "{}",
            streams.stdout
        );
    }

    /// The human rendering of the same answer, which is what a person at a
    /// terminal gets. `writable: false`, not "no" — the two renderings may not
    /// disagree.
    #[test]
    fn the_human_rendering_of_an_explanation_says_what_the_json_says() {
        let text = run_on_board(
            "rivet",
            &argv(&["--output", "text", "explain", "/etc/shells"]),
            true,
        )
        .stdout;
        assert!(text.contains("kind: view"), "{text}");
        assert!(text.contains("writable: false"), "{text}");
        assert!(text.contains("change-with:"), "{text}");
    }

    // -- measure, end to end ---------------------------------------------------

    /// **`ADR-029` §7.2's measurement, over the transport it will actually be
    /// taken through.** A remote `rivet measure hash /dev/mmcblk0p3` is the
    /// only shape available, because this binary is the login shell and refuses
    /// every other program — so the `-c` path is what is
    /// asserted here rather than a bare `argv`.
    ///
    /// It hashes this crate's own source file, which is a path that exists on any
    /// machine running this test and is emphatically not 640 MiB. **The numbers
    /// this produces mean nothing** (`AGENTS.md` §6); what is asserted is that the
    /// request reaches the measurement and comes back as a value with every field
    /// the ruling asked for.
    #[test]
    fn measuring_a_hash_remotely_produces_a_value_with_both_algorithms() {
        let this_file = concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs");
        let command = format!("rivet measure hash {this_file}");
        let streams = piped("rivet", &["-c", &command]);

        // On a platform with no `clock_gettime(CLOCK_MONOTONIC)` this refuses
        // rather than producing an untimed benchmark, which is the behaviour
        // `crate::measure`'s `NoClock` exists for. Assert the refusal is the
        // right one and stop; on Linux, assert the whole value.
        if streams.code != 0 {
            assert!(
                streams.stderr.contains("E_MACHINE_REFUSED"),
                "measure failed for a reason other than the missing clock: {}",
                streams.stderr
            );
            assert_eq!(streams.stdout, "", "nothing on stdout");
            return;
        }

        assert_eq!(streams.stderr, "");
        for field in [
            r#""bytes":"#,
            r#""buffer-bytes":"#,
            r#""read-elapsed-ms":"#,
            r#""elapsed-ms":"#,
            r#""algorithm":"blake3""#,
            r#""algorithm":"sha256""#,
            r#""throughput-mb-s":"#,
            r#""digest":"#,
        ] {
            assert!(
                streams.stdout.contains(field),
                "{field} missing: {}",
                streams.stdout
            );
        }
        // The path is echoed back verbatim, so a report pasted into the record
        // says what was measured (`AGENTS.md` §5).
        assert!(streams.stdout.contains(this_file), "{}", streams.stdout);
    }

    /// The human rendering says what the machine form says — `ADR-006` §4, over a
    /// command that produces numbers rather than strings.
    #[test]
    fn the_human_rendering_of_a_measurement_says_what_the_json_says() {
        let this_file = concat!(env!("CARGO_MANIFEST_DIR"), "/src/cli.rs");
        let text = run_on_board(
            "rivet",
            &argv(&["--output", "text", "measure", "hash", this_file]),
            true,
        );
        if text.code != 0 {
            assert!(text.stderr.contains("E_MACHINE_REFUSED"), "{}", text.stderr);
            return;
        }
        for needle in [
            "bytes:",
            "algorithm: blake3",
            "algorithm: sha256",
            "digest:",
        ] {
            assert!(
                text.stdout.contains(needle),
                "{needle} missing: {}",
                text.stdout
            );
        }
    }

    /// **`measure` is not in the operation catalogue and must not appear as
    /// though it were.** `rivet --version` is the authority for what this build
    /// implements (`ADR-006` §7), and a bare word that turned up there would be
    /// claiming a place in a vocabulary the maintainer has not granted it.
    #[test]
    fn measure_is_absent_from_the_published_catalogue_and_from_the_verb_set() {
        let json = run_on_board("rivet", &argv(&["--output", "json", "--version"]), false).stdout;
        assert!(
            !json.contains(r#""verb":"measure""#),
            "measure is published as a verb: {json}"
        );
        for op in operation::CATALOGUE {
            assert_ne!(op.verb.as_str(), "measure");
        }
        // ...and the usage still tells an operator it exists, because a command
        // reachable only by reading the source is one nobody will run.
        assert!(USAGE.contains("rivet measure hash"), "{USAGE}");
    }

    // -- schema ----------------------------------------------------------------

    /// `ADR-006` §7's other half, and `ADR-007` §3.2's document schema —
    /// `rivet get schema` since 2026-08-25, when the bare word was retired.
    #[test]
    fn schema_emits_the_configuration_documents_json_schema() {
        let streams = piped("rivet", &["get", "schema"]);
        assert_eq!(streams.code, 0, "stderr: {}", streams.stderr);
        assert_eq!(streams.stderr, "");
        assert!(streams.stdout.starts_with('{'), "{}", streams.stdout);
        assert!(
            streams
                .stdout
                .contains(r#""$schema":"https://json-schema.org/"#),
            "{}",
            streams.stdout
        );
        // Every top-level key of the document, from the schema crate's own list
        // rather than from a literal here.
        for key in rivet_config::Document::DECLARED_KEYS {
            assert!(
                streams.stdout.contains(&format!(r#""{key}""#)),
                "{key} missing"
            );
        }
    }

    /// **The honesty clause reaches the operator**, rather than living in a
    /// module comment: JSON Schema cannot express six of `rivet-config`'s
    /// refusals, and a schema that stayed quiet would tell an agent its
    /// document was valid when the machine will refuse it.
    #[test]
    fn the_emitted_schema_says_that_passing_it_is_not_the_same_as_being_accepted() {
        let out = piped("rivet", &["get", "schema"]).stdout;
        assert!(out.contains("NOT sufficient"), "{out}");
        assert!(out.contains("rivet-config is the authority"), "{out}");
    }

    /// The schema version the CLI reports and the one the schema pins are the
    /// same number, because both come from `rivet_config::SCHEMA_VERSION`.
    /// Two answers to "which schema does this build read" is `ADR-011` §3.
    #[test]
    fn the_version_and_the_schema_agree_about_which_schema_this_build_reads() {
        let version =
            run_on_board("rivet", &argv(&["--output", "json", "--version"]), false).stdout;
        let schema = piped("rivet", &["get", "schema"]).stdout;
        let declared = rivet_config::SCHEMA_VERSION.get();
        assert!(
            version.contains(&format!(r#""schema-version":{declared}"#)),
            "{version}"
        );
        assert!(
            schema.contains(&format!(r#""const":{declared}"#)),
            "{schema}"
        );
    }

    // -- help ------------------------------------------------------------------

    #[test]
    fn help_goes_to_stdout_and_exits_zero_because_it_was_asked_for() {
        let streams = piped("rivet", &["--help"]);
        assert_eq!(streams.code, 0);
        assert_eq!(streams.stderr, "");
        assert!(streams.stdout.contains("rivet"));
    }

    /// The usage text is the only prose a person reads before they know
    /// anything, so it has to carry the facts that are surprising: which verbs
    /// exist, that almost none is implemented, and that this is not an
    /// interactive shell.
    #[test]
    fn the_usage_names_every_verb_and_says_which_of_them_this_build_implements() {
        assert!(USAGE.contains("E_NO_INTERACTIVE_SHELL"), "{USAGE}");
        assert!(USAGE.contains("E_NOT_A_SHELL"), "{USAGE}");
        assert!(USAGE.contains("E_UNIMPLEMENTED"), "{USAGE}");
        for verb in Verb::ALL {
            assert!(USAGE.contains(verb.as_str()), "usage does not list {verb}");
        }
        // Every implemented verb is shown with its own line, so a reader knows
        // what to type without running --version first.
        for op in operation::CATALOGUE {
            assert!(
                USAGE.contains(&format!("rivet {} ", op.verb)),
                "usage does not show how to run {}",
                op.verb
            );
        }
        assert!(USAGE.contains("rivet schema"), "{USAGE}");
    }

    /// Every exit code the module documentation of [`refusal`] tabulates is
    /// reachable, and reachable from the invocation that the table claims
    /// produces it. A documented code nothing emits is documentation that is
    /// wrong.
    #[test]
    fn each_documented_exit_code_is_produced_by_the_invocation_it_is_documented_for() {
        assert_eq!(piped("rivet", &["--version"]).code, 0);
        assert_eq!(piped("rivet", &["list"]).code, 1);
        assert_eq!(piped("rivet", &["explain", "/etc/fstab"]).code, 1);
        assert_eq!(piped("rivet", &["--wat"]).code, 2);
        assert_eq!(piped("rivet", &["explain"]).code, 2);
        assert_eq!(piped("-rivet", &[]).code, 126);
        assert_eq!(piped("rivet", &["-c", "/usr/lib/sftp-server"]).code, 127);
    }

    // -- `ADR-006` §8.10's file surface, through `argv` ----------------------

    /// **`rivet get file` reaches the same file `apply edit` changes**, which is
    /// the asymmetry §8.10.5 was ruled to close — *"an agent can change a file
    /// it cannot read back."*
    ///
    /// It is a whole-binary test rather than a domain one because the thing
    /// being asserted is the wiring: the subject reaches the `Editor` port
    /// through the same board the write half uses, and the answer comes back as
    /// JSON on stdout with exit 0.
    #[test]
    fn get_file_reads_back_the_file_apply_edit_changes() {
        let mut board = FakeBoard::new();
        let out = run(
            "rivet",
            &argv(&["get", "file", FakeBoard::EDITABLE]),
            false,
            &mut board,
        );
        assert_eq!(out.code, 0, "{}", out.stderr);
        assert!(out.stdout.contains("\"present\":true"), "{}", out.stdout);
        assert!(
            out.stdout.contains("show_banner = true"),
            "the contents did not come back: {}",
            out.stdout
        );
        // And the same command says whether the writing subjects would accept
        // it, without writing anything.
        assert!(out.stdout.contains("\"changeable\""), "{}", out.stdout);
    }

    /// A path with nothing at it is `Ok` and says so — `get desired-state`'s
    /// precedent, and the case a caller hits before creating something.
    #[test]
    fn get_file_on_a_path_with_nothing_at_it_exits_zero_and_says_present_false() {
        let out = piped("rivet", &["get", "file", "/state/not-here"]);
        assert_eq!(out.code, 0, "{}", out.stderr);
        assert!(out.stdout.contains("\"present\":false"), "{}", out.stdout);
    }

    /// **`apply remove-file` needs the word, and the plan comes back first.**
    /// `ADR-012` §7.1 through `argv`.
    #[test]
    fn apply_remove_file_returns_its_plan_and_removes_nothing_without_the_word() {
        let mut board = FakeBoard::new();
        let out = run(
            "rivet",
            &argv(&["apply", "remove-file", FakeBoard::EDITABLE]),
            false,
            &mut board,
        );
        assert_eq!(out.code, 1, "{}", out.stdout);
        assert!(
            out.stderr.contains("E_CONFIRMATION_REQUIRED"),
            "{}",
            out.stderr
        );
        assert!(out.stderr.contains("CANNOT BE UNDONE"), "{}", out.stderr);
        assert!(
            board.edits.store().text(FakeBoard::EDITABLE).is_some(),
            "the plan removed the file"
        );

        let done = run(
            "rivet",
            &argv(&["apply", "remove-file", FakeBoard::EDITABLE, "confirm"]),
            false,
            &mut board,
        );
        assert_eq!(done.code, 0, "{}", done.stderr);
        assert!(
            board.edits.store().text(FakeBoard::EDITABLE).is_none(),
            "the file is still there"
        );
        // **The removal is journalled at `warn`**, which is what makes a
        // destroyed file findable in a journal an operator is scanning.
        let journal = board.edits.store().journal();
        assert!(
            journal.iter().any(|line| line.contains("remove-file")),
            "no record of the removal: {journal:?}"
        );
        assert!(
            journal.iter().any(|line| line.contains("\"sev\":\"warn\"")),
            "the removal was recorded as routine: {journal:?}"
        );
    }

    /// The whole family is reachable through `argv` and refuses a path outside
    /// the permitted root — the gate, asserted where an operator meets it.
    #[test]
    fn every_file_management_subject_is_reachable_and_refuses_a_path_outside_state() {
        for words in [
            vec!["apply", "create-file", "/system/x", "confirm"],
            vec!["apply", "create-directory", "/system/x", "confirm"],
            vec!["apply", "set-mode", "/system/x", "0600", "confirm"],
            vec!["apply", "set-owner", "/system/x", "root", "confirm"],
            vec!["apply", "rename", "/system/x", "/system/y", "confirm"],
            vec!["apply", "remove-file", "/system/x", "confirm"],
            vec!["apply", "remove-directory", "/system/x", "confirm"],
        ] {
            let out = piped("rivet", &words);
            assert_eq!(out.code, 1, "{words:?}: {}", out.stdout);
            assert!(out.stderr.contains("E_UNSAFE"), "{words:?}: {}", out.stderr);
            assert!(
                out.stderr.contains("/state"),
                "{words:?} does not name the permitted root: {}",
                out.stderr
            );
        }
    }
}
