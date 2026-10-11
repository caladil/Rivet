//! `cargo rivet doctor` — **what this workstation has, and what it is missing,
//! for each thing a person comes here to do.** `ADR-062` §3.
//!
//! # Why a verb and not a README table
//!
//! What a workstation needs is a fact about *that* workstation: which toolchain
//! is active in the directory you are standing in, whether Docker's daemon is
//! up, whether the Buildroot volume was ever populated, whether the public keys
//! in the checkout agree with the seeds under `~/.rivet`. A table in prose can
//! say which of those matter; only a command can say which of them are true
//! here. So the README names the three goals and points at this, and this is
//! the authority on the rest.
//!
//! # Three goals, because the requirements genuinely differ
//!
//! | goal | what it is | what it needs that the one above does not |
//! | :-- | :-- | :-- |
//! | `run` | operate a Rivet: `run`, `deploy`, `install`, `provision`, `write-image` | the two workstation binaries and the credentials |
//! | `app` | build and sign an application with the SDK | the musl target, `rust-lld`, the application seed |
//! | `image` | build a card image or a system update | a checkout, a container runtime, the Buildroot volume, the firmware |
//!
//! A person who only wants to reach a board should not be told they are
//! broken because Docker is not installed, so each goal is judged on its own
//! and `--for` narrows the report to the ones asked about.
//!
//! # It is read in a glance, and explains itself only when asked
//!
//! The report opens with one verdict per goal. Under each, everything that is
//! fine is one short line, and anything that is not is a headline, at most one
//! sentence of consequence, and the commands that fix it. **The reasoning
//! behind each finding exists, and is printed only with `--verbose`**: a
//! paragraph under every line is what makes a checklist unreadable, and the
//! person who needs the paragraph is the one who asks for it.
//!
//! # It changes nothing
//!
//! Every check reads. Nothing is installed, minted, pulled or started — a
//! doctor that fixed things would be `setup`, and `setup` already exists for
//! the half of this that is safe to automate. Each finding names the command
//! that fixes it instead.
//!
//! # Every probe goes through [`Host`]
//!
//! The verdicts are the part worth testing and the probes are the part a test
//! cannot reproduce — a Docker daemon, a rustup, a `PATH`. So [`evaluate`] is
//! pure over a [`Host`], the real one shells out, and the tests hand it fixed
//! answers for each shape of machine this has to describe.

use std::path::{Path, PathBuf};

use crate::credentials::{self, Need, Overrides, Survey};

/// The triple every Rivet binary is built for — `ADR-003` §4.
const TARGET: &str = "aarch64-unknown-linux-musl";

/// The Docker volume `tools/container/build-card.sh` reads the Buildroot output
/// from, unless `RIVET_BR_VOLUME` names another.
const DEFAULT_BR_VOLUME: &str = "rivet-br";

/// The files a card cannot be built without, relative to a checkout.
/// Gitignored on purpose — `ADR-010` §6.
const FIRMWARE: &[&str] = &["vendor/boot/start4.elf", "vendor/boot/fixup4.dat"];

/// Where `--verbose` wraps the reasoning.
const WIDTH: usize = 78;

pub fn usage() -> String {
    "usage: cargo rivet doctor [--for run|app|image]... [--verbose]\n\
     \n\
     Checks this workstation against each thing you might be here to do, and\n\
     names the command that fixes each gap. It reads and changes nothing.\n\
     \n\
     \x20 run    operate a Rivet — run, deploy, install, provision, write-image\n\
     \x20 app    build and sign an application with the SDK (cargo rivet package)\n\
     \x20 image  build a card image or a system update (tools/container/*)\n\
     \n\
     \x20 --for GOAL     check only this goal; repeatable. Default: all three\n\
     \x20 -v, --verbose  also print why each finding matters, and every path\n\
     \n\
     \x20 ✓ fine    ! works, with a caveat    ✗ must be fixed    · not needed\n\
     \x20 unless you need it\n\
     \n\
     Exits non-zero when anything marked ✗ is found. Run it from the directory\n\
     you will build in: the active Rust toolchain, and whether you are in a\n\
     Rivet checkout, both depend on it.\n"
        .to_owned()
}

/// A thing a person comes here to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Goal {
    Run,
    App,
    Image,
}

impl Goal {
    const ALL: [Goal; 3] = [Goal::Run, Goal::App, Goal::Image];

    fn word(self) -> &'static str {
        match self {
            Goal::Run => "run",
            Goal::App => "app",
            Goal::Image => "image",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Goal::Run => "Run a Rivet",
            Goal::App => "Build applications",
            Goal::Image => "Build card images and updates",
        }
    }
}

#[derive(Debug)]
pub struct DoctorArgs {
    pub goals: Vec<Goal>,
    pub verbose: bool,
}

/// # Errors
///
/// An unknown option or goal.
pub fn parse(args: &[String]) -> Result<DoctorArgs, String> {
    let mut goals = Vec::new();
    let mut verbose = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--for" => {
                let word = args.get(i + 1).ok_or("--for needs a goal: run, app or image")?;
                let goal = Goal::ALL
                    .into_iter()
                    .find(|g| g.word() == word)
                    .ok_or_else(|| format!("{word:?} is not a goal; the goals are run, app and image"))?;
                if !goals.contains(&goal) {
                    goals.push(goal);
                }
                i += 2;
            }
            "-v" | "--verbose" => {
                verbose = true;
                i += 1;
            }
            other => return Err(format!("{other:?} is not an option of `doctor`\n\n{}", usage())),
        }
    }
    if goals.is_empty() {
        goals = Goal::ALL.to_vec();
    }
    Ok(DoctorArgs { goals, verbose })
}

/// How one check came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    /// Works, with a caveat worth knowing. Not a blocker.
    Warn,
    /// The goal cannot be done until this is fixed.
    Fail,
    /// Absent, and needed only in a case the finding names.
    Optional,
    /// Not evaluated, because something it depends on already failed.
    Skip,
}

impl Status {
    fn mark(self) -> &'static str {
        match self {
            Status::Ok => "✓",
            Status::Warn => "!",
            Status::Fail => "✗",
            Status::Optional => "·",
            Status::Skip => "-",
        }
    }
}

/// One line of the report, and what may appear under it.
#[derive(Debug, Clone)]
pub struct Finding {
    pub status: Status,
    /// The line itself: short, and a statement of fact.
    pub what: String,
    /// At most one sentence of consequence, printed under a line that is not
    /// fine.
    pub note: Option<String>,
    /// The commands that fix it, one per line.
    pub fixes: Vec<String>,
    /// Why it matters, printed only with `--verbose`.
    pub why: String,
}

impl Finding {
    fn new(status: Status, what: impl Into<String>) -> Self {
        Finding { status, what: what.into(), note: None, fixes: Vec::new(), why: String::new() }
    }

    fn note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    fn fix(mut self, fix: impl Into<String>) -> Self {
        self.fixes.push(fix.into());
        self
    }

    fn why(mut self, why: impl Into<String>) -> Self {
        self.why = why.into();
        self
    }
}

fn ok(what: impl Into<String>) -> Finding {
    Finding::new(Status::Ok, what)
}

/// What a check may ask of the machine. The real one is [`RealHost`].
pub trait Host {
    /// `std::env::consts::OS` and `ARCH`, as parameters so a test can be a Mac.
    fn os(&self) -> &str;
    fn arch(&self) -> &str;
    /// An environment variable that is set and not empty.
    fn env(&self, name: &str) -> Option<String>;
    /// The first `program` on `PATH`, as a shell would resolve it.
    fn which(&self, program: &str) -> Option<PathBuf>;
    /// Run a program and return its trimmed stdout. `Err` carries why it did
    /// not start or did not succeed, in a form short enough to quote.
    fn run(&self, program: &str, args: &[&str], cwd: Option<&Path>) -> Result<String, String>;
    fn is_file(&self, path: &Path) -> bool;
    /// A file's text, or `None` when it cannot be read.
    fn read(&self, path: &Path) -> Option<String>;
    fn cwd(&self) -> Option<PathBuf>;
    /// `rivet-provision` beside this binary — where `cargo install` puts it.
    fn provision_sibling(&self) -> Option<PathBuf>;
    /// This binary, and the `cargo-rivet` an unqualified `cargo rivet` runs.
    fn this_binary(&self) -> Option<PathBuf>;
    fn cargo_rivet_on_path(&self) -> Option<PathBuf>;
    fn same_file(&self, a: &Path, b: &Path) -> bool;
    /// Whether a seed's public half is the key written at `published`.
    fn seed_agrees(&self, seed: &Path, published: &Path) -> Result<bool, String>;
}

/// The directory holding a Rivet checkout, walking up from `from`.
///
/// Recognised by the crate this binary is built from, which only a checkout
/// of this repository carries.
fn checkout_from(host: &dyn Host, from: &Path) -> Option<PathBuf> {
    from.ancestors()
        .find(|dir| {
            host.is_file(&dir.join("cmd").join("cargo-rivet").join("Cargo.toml"))
                && host.is_file(&dir.join("rivet-core").join("Cargo.toml"))
        })
        .map(Path::to_path_buf)
}

/// The whole report, as findings grouped by goal. Pure over `host`.
///
/// `at` is the credentials root (`~/.rivet`), handed in so a test can point
/// the credential survey at a scratch directory.
pub fn evaluate(
    host: &dyn Host,
    goals: &[Goal],
    at: Option<&Path>,
    over: &Overrides,
    this_build: &str,
) -> Vec<(Goal, Vec<Finding>)> {
    let cwd = host.cwd();
    let checkout = cwd.as_deref().and_then(|c| checkout_from(host, c));
    let home = host.env("HOME").or_else(|| host.env("USERPROFILE")).map(PathBuf::from);
    let ctx = Context {
        host,
        at,
        over,
        this_build,
        cwd: cwd.as_deref(),
        checkout: checkout.as_deref(),
        home: home.as_deref(),
    };
    goals
        .iter()
        .map(|&goal| {
            let findings = match goal {
                Goal::Run => ctx.run_goal(),
                Goal::App => ctx.app_goal(),
                Goal::Image => ctx.image_goal(),
            };
            (goal, findings)
        })
        .collect()
}

struct Context<'a> {
    host: &'a dyn Host,
    at: Option<&'a Path>,
    over: &'a Overrides,
    this_build: &'a str,
    cwd: Option<&'a Path>,
    checkout: Option<&'a Path>,
    home: Option<&'a Path>,
}

impl Context<'_> {
    /// A path as a person reads it: relative inside the checkout, `~/…` under
    /// the home directory, and whole otherwise.
    fn short(&self, path: &Path) -> String {
        if let Some(rest) = self.checkout.and_then(|c| path.strip_prefix(c).ok()) {
            return rest.display().to_string();
        }
        if let Some(rest) = self.home.and_then(|h| path.strip_prefix(h).ok()) {
            return format!("~/{}", rest.display());
        }
        path.display().to_string()
    }

    // --- run ---------------------------------------------------------------

    fn run_goal(&self) -> Vec<Finding> {
        let mut out = vec![self.this_tool(), self.provision_pair()];
        out.extend(self.credentials(&[]));
        out.push(self.card_writing());
        out
    }

    /// Whether `cargo rivet` resolves to the binary answering. A shadowed or
    /// absent one is a caveat rather than a failure: running this file by path
    /// works, and the finding exists so nobody is surprised that `cargo rivet`
    /// does something else.
    fn this_tool(&self) -> Finding {
        let install = "cargo install --path cmd/cargo-rivet --force --locked";
        let (Some(me), on_path) = (self.host.this_binary(), self.host.cargo_rivet_on_path()) else {
            return ok("cargo-rivet");
        };
        match on_path {
            Some(first) if self.host.same_file(&me, &first) => ok("cargo-rivet, installed"),
            Some(first) => Finding::new(Status::Warn, "`cargo rivet` runs a different cargo-rivet")
                .note(format!("It runs {}; this one is {}.", self.short(&first), self.short(&me)))
                .fix(install),
            None => Finding::new(Status::Warn, "cargo-rivet is not installed")
                .note(format!("This one is {}.", self.short(&me)))
                .fix(install),
        }
    }

    /// `provision` runs `rivet-provision` as its interview and refuses a pair
    /// whose builds disagree (`.rulings/` R-191), so the pair is judged here
    /// the same way, with the same comparison.
    fn provision_pair(&self) -> Finding {
        let install = "cargo install --path cmd/rivet-provision --force --locked";
        let why = "`cargo rivet provision` runs rivet-provision as its interview, and refuses a pair \
                   whose builds disagree because the two share the interview's files and its \
                   notebook format.";
        let Some(sibling) = self.host.provision_sibling() else {
            return Finding::new(Status::Fail, "rivet-provision is not installed").fix(install).why(why);
        };
        let said = self.host.run(&sibling.to_string_lossy(), &["--version"], None);
        let theirs = said.as_deref().ok().and_then(|text| {
            let line = text.lines().find(|l| !l.trim().is_empty())?.trim();
            line.split_once(' ').map(|(_, build)| build.trim().to_owned())
        });
        match theirs {
            Some(build) if crate::provision::comparable(&build) == crate::provision::comparable(self.this_build) => {
                ok("rivet-provision, the same build")
            }
            Some(build) => Finding::new(Status::Fail, "rivet-provision is a different build")
                .note(format!("It is {build}; cargo-rivet is {}.", self.this_build))
                .fix("cargo install --path cmd/cargo-rivet --force --locked")
                .fix(install)
                .why(why),
            None => Finding::new(Status::Fail, "rivet-provision will not say which build it is")
                .fix(install)
                .why(why),
        }
    }

    /// The credential survey `setup --check` prints, reduced to verdicts: the
    /// ones that are present collapse into one line, and each one that is not
    /// gets its own. A `Required` credential missing is a failure; an
    /// `Advised` one, a caveat.
    ///
    /// `only` narrows it to credentials whose names contain one of the words;
    /// empty means all of them.
    fn credentials(&self, only: &[&str]) -> Vec<Finding> {
        let Some(at) = self.at else {
            return vec![Finding::new(Status::Fail, "no home directory, so no ~/.rivet")
                .note("This host sets neither HOME nor USERPROFILE.")];
        };
        let items: Vec<_> = Survey::take(at, self.over)
            .items
            .into_iter()
            .filter(|c| only.is_empty() || only.iter().any(|w| c.name.contains(w)))
            .collect();

        let (present, absent): (Vec<_>, Vec<_>) =
            items.into_iter().partition(|c| c.present && c.incomplete.is_none());
        let mut out = Vec::new();
        if let [one] = present.as_slice() {
            out.push(ok(one.name).why(format!("At {}.", self.short(&one.path))));
        } else if !present.is_empty() {
            // Counted by kind, because five names on one line is wider than a
            // terminal; `--verbose` names each and says where it is.
            let count = |shapes: &[credentials::Shape]| present.iter().filter(|c| shapes.contains(&c.shape)).count();
            let seeds = count(&[credentials::Shape::Seed]);
            let identities = count(&[credentials::Shape::Identity, credentials::Shape::EnrolledIdentity]);
            let authority = count(&[credentials::Shape::Authority]);
            let mut parts = Vec::new();
            match seeds {
                0 => {}
                1 => parts.push("1 signing seed".to_owned()),
                n => parts.push(format!("{n} signing seeds")),
            }
            match identities {
                0 => {}
                1 => parts.push("1 identity".to_owned()),
                n => parts.push(format!("{n} identities")),
            }
            if authority > 0 {
                parts.push("fleet authority".to_owned());
            }
            let each: Vec<String> =
                present.iter().map(|c| format!("{} at {}", c.name, self.short(&c.path))).collect();
            out.push(ok(format!("{}: {}", self.short(at), parts.join(", "))).why(format!("{}.", each.join("; "))));
        }
        for c in absent {
            let (status, what) = if c.need == Need::Required {
                (Status::Fail, format!("no {}", c.name))
            } else {
                (Status::Optional, format!("no {}", c.name))
            };
            let mut f = Finding::new(status, what).fix(c.how.clone()).why(c.why);
            if let Some(part) = &c.incomplete {
                f.note = Some(format!("{} is missing {part}.", self.short(&c.path)));
            }
            out.push(f);
        }
        out
    }

    /// `write-image --device` enumerates removable disks on Windows, Linux and
    /// macOS (`crate::enumerate`), and refuses a device it cannot check. Without
    /// `--device` it still leaves a finished image an imaging tool can write.
    fn card_writing(&self) -> Finding {
        match self.host.os() {
            "windows" => ok("can write cards (asks for administrator when it writes)"),
            "linux" => ok("can write cards"),
            "macos" => ok("can write cards (sudo asks for your password when it writes)"),
            _ => Finding::new(Status::Warn, "cannot write a card directly on this system")
                .note("Use write-image without --device, then Raspberry Pi Imager.")
                .why(
                    "Removable-disk enumeration exists for Windows, Linux and macOS, and a device \
                     this tool cannot identify as removable is refused rather than trusted. \
                     Without --device, write-image still injects the document into both boot \
                     partitions and leaves a finished image.",
                ),
        }
    }

    // --- app ---------------------------------------------------------------

    fn app_goal(&self) -> Vec<Finding> {
        let mut out = Vec::new();
        if self.host.which("rustup").is_none() {
            out.push(Finding::new(Status::Fail, "rustup is not installed").fix("https://rustup.rs"));
            out.push(Finding::new(Status::Skip, format!("{TARGET} target (needs rustup)")));
        } else {
            out.push(self.target_installed());
        }
        out.push(self.linker());
        out.extend(self.credentials(&["application"]));
        out.push(self.c_compiler());
        out
    }

    /// The target must be installed on the toolchain that builds the crate,
    /// which is whatever is active in the working directory — inside this
    /// checkout that is `rust-toolchain.toml`'s pin, outside it usually
    /// `stable`, and the two have separate target lists.
    fn target_installed(&self) -> Finding {
        let cwd = self.cwd;
        let toolchain = self
            .host
            .run("rustup", &["show", "active-toolchain"], cwd)
            .ok()
            .and_then(|s| s.split_whitespace().next().map(str::to_owned))
            .unwrap_or_else(|| "the active toolchain".to_owned());
        let why = "The target must be installed on the toolchain that builds the crate, which \
                   is whatever is active in this directory: inside a Rivet checkout, \
                   rust-toolchain.toml's pin, which installs it; elsewhere usually `stable`, \
                   whose target list is separate.";
        match self.host.run("rustup", &["target", "list", "--installed"], cwd) {
            Ok(list) if list.lines().any(|l| l.trim() == TARGET) => {
                ok(format!("{TARGET} target ({toolchain})"))
            }
            Ok(_) => Finding::new(Status::Fail, format!("no {TARGET} target on {toolchain}"))
                .fix(format!("rustup target add {TARGET} --toolchain {toolchain}"))
                .why(why),
            Err(e) => Finding::new(Status::Fail, format!("rustup will not list targets: {e}")),
        }
    }

    /// `rust-lld` ships inside the Rust toolchain, so it is never missing — what
    /// goes missing is the setting that selects it. This checkout's
    /// `.cargo/config.toml` sets it for everything under it; a crate elsewhere
    /// inherits nothing, and the host's own `cc` then tries to link a Linux
    /// binary and fails naming linker options rather than the cause.
    fn linker(&self) -> Finding {
        let var = format!("CARGO_TARGET_{}_LINKER", TARGET.to_uppercase().replace('-', "_"));
        if let Some(linker) = self.host.env(&var) {
            return ok(format!("{linker} is the linker (${var})"));
        }
        let configured = self.cwd.and_then(|cwd| {
            cwd.ancestors()
                .flat_map(|dir| [dir.join(".cargo").join("config.toml"), dir.join(".cargo").join("config")])
                .find(|file| {
                    self.host.read(file).is_some_and(|text| {
                        text.contains(&format!("[target.{TARGET}]")) && text.contains("rust-lld")
                    })
                })
        });
        match configured {
            Some(file) => ok(format!("rust-lld is the linker ({})", self.short(&file))),
            None => Finding::new(Status::Warn, "nothing here selects rust-lld as the linker")
                .note("A crate outside a Rivet checkout needs this in its .cargo/config.toml:")
                .fix(format!("[target.{TARGET}] linker = \"rust-lld\""))
                .why(
                    "Without it the host's own cc is asked to link a Linux binary, and fails \
                     naming linker options rather than the cause. A crate inside a Rivet \
                     checkout inherits the setting; the Rust templates carry their own.",
                ),
        }
    }

    /// Only a crate whose dependencies compile C needs this — the SDK itself
    /// does not. Buildroot's cross compiler is a Linux x86-64 program, so off a
    /// Linux x86-64 host it runs only inside the build container.
    fn c_compiler(&self) -> Finding {
        let var = format!("CC_{}", TARGET.replace('-', "_"));
        if let Some(cc) = self.host.env(&var) {
            return ok(format!("C cross compiler ({cc})"));
        }
        Finding::new(Status::Optional, "no C cross compiler — only crates that compile C need one")
            .why(
                "Pure-Rust applications, including everything the SDK provides, need nothing \
                 more. A crate that compiles C (rusqlite's `bundled`, ring) builds inside the \
                 container, where tools/toolchain/env.sh sets the compiler: \
                 RIVET_APP_SRC=<crate repo> tools/container/exec.sh sh -c '. \
                 tools/toolchain/env.sh && cargo rivet package …'. `--for image` says whether \
                 the container is ready.",
            )
    }

    // --- image -------------------------------------------------------------

    fn image_goal(&self) -> Vec<Finding> {
        let mut out = Vec::new();

        let Some(checkout) = self.checkout else {
            out.push(
                Finding::new(Status::Fail, "not in a Rivet checkout")
                    .note("Cards and updates are built from one; run this from inside it."),
            );
            return out;
        };
        out.push(ok(format!("Rivet checkout ({})", self.short_home(checkout))));

        if self.host.which("git").is_none() {
            out.push(
                Finding::new(Status::Fail, "git is not installed")
                    .note("tools/card/bump.sh mints each version with it."),
            );
        }

        let missing: Vec<&str> =
            FIRMWARE.iter().copied().filter(|f| !self.host.is_file(&checkout.join(f))).collect();
        out.push(if missing.is_empty() {
            ok("Raspberry Pi boot firmware")
        } else {
            Finding::new(Status::Fail, "no Raspberry Pi boot firmware")
                .note(format!("Missing {}.", missing.join(", ")))
                .fix("tools/fetch-firmware.sh")
        });

        out.extend(self.credentials(&["system"]));
        out.extend(self.trust_keys(checkout));
        out.extend(self.container(checkout));

        if self.host.which("uv").is_none() {
            out.push(
                Finding::new(Status::Warn, "uv is not installed")
                    .note("The documentation guards and the pre-commit hook need it.")
                    .fix("https://docs.astral.sh/uv/"),
            );
        }
        out
    }

    /// The checkout's own path, which [`Self::short`] would render as empty.
    fn short_home(&self, path: &Path) -> String {
        match self.home.and_then(|h| path.strip_prefix(h).ok()) {
            Some(rest) => format!("~/{}", rest.display()),
            None => path.display().to_string(),
        }
    }

    /// A card trusts the public keys under `keys/` when it is built, and a
    /// board refuses everything signed with any other key for the rest of its
    /// life — so a checkout whose committed keys are somebody else's builds a
    /// card that refuses its own builder, with nothing failing at build time.
    fn trust_keys(&self, checkout: &Path) -> Vec<Finding> {
        let pairs = [
            ("system", "rivet-system.pub", credentials::system_seed(self.over, self.at)),
            ("application", "rivet-app.pub", credentials::app_seed(self.over, self.at)),
        ];
        let mut agree = Vec::new();
        let mut out = Vec::new();
        for (which, file, seed) in pairs {
            let Ok(seed) = seed else { continue };
            let published = checkout.join("keys").join(file);
            match self.host.seed_agrees(&seed, &published) {
                Ok(true) => agree.push(format!("keys/{file}")),
                Ok(false) => out.push(
                    Finding::new(Status::Fail, format!("keys/{file} is not your {which} key"))
                        .note(
                            "Cards built here would refuse everything this machine signs. Restore \
                             the seed that matches it, or make this machine's seed the trusted one:",
                        )
                        .fix("cargo rivet key restore DIR")
                        .fix(format!("cargo rivet key {} --out keys/{file}", self.short(&seed)))
                        .why(
                            "A card trusts the committed public keys when it is built, and every \
                             board built from it refuses anything signed by another key for the \
                             rest of its life. The two fixes are not equivalent. Restoring keeps \
                             every existing board: run `cargo rivet key backup DIR` on the \
                             machine that holds the matching seed, then `key restore DIR` here. \
                             Rewriting the committed key means every card built afterwards trusts \
                             this machine's seed, and no existing board does.",
                        ),
                ),
                Err(e) => out.push(Finding::new(Status::Fail, format!("keys/{file}: {e}"))),
            }
        }
        if !agree.is_empty() {
            out.insert(0, ok(format!("{} match your seeds", agree.join(" and "))));
        }
        out
    }

    /// The runtime, its daemon, the platform it runs, the build image and the
    /// Buildroot volume — each depends on the one before, so the first failure
    /// stops the chain rather than reporting five symptoms of one cause.
    fn container(&self, checkout: &Path) -> Vec<Finding> {
        let mut out = Vec::new();
        let docker = self.host.env("RIVET_DOCKER").unwrap_or_else(|| "docker".to_owned());
        let docker = docker.split_whitespace().next().unwrap_or("docker").to_owned();
        let br_volume = self.host.env("RIVET_BR_VOLUME").unwrap_or_else(|| DEFAULT_BR_VOLUME.to_owned());

        if self.host.which(&docker).is_none() {
            out.push(
                Finding::new(Status::Fail, format!("{docker} is not installed"))
                    .note("Docker Desktop on macOS and Windows; Docker Engine or Podman on Linux.")
                    .why("RIVET_DOCKER=podman selects Podman."),
            );
            return out;
        }
        let server_arch = match self.host.run(&docker, &["version", "--format", "{{.Server.Arch}}"], None) {
            Ok(arch) => arch,
            Err(e) => {
                out.push(
                    Finding::new(Status::Fail, format!("{docker} is not running"))
                        .note("Start Docker Desktop, or the Docker or Podman service.")
                        .why(format!("It said: {e}")),
                );
                return out;
            }
        };

        // The images are x86-64 only (ADR-062 §4). On any other daemon they
        // run emulated, which works and is slower.
        out.push(if server_arch == "amd64" || server_arch == "x86_64" {
            ok(docker.clone())
        } else {
            ok(format!("{docker} (x86-64 images run emulated on this {server_arch} host)")).why(
                "The build images exist only for x86-64. Docker Desktop runs them through Rosetta \
                 when \"Use Rosetta for x86_64/amd64 emulation\" is on, and through QEMU, several \
                 times slower, when it is off.",
            )
        });

        let version = self
            .host
            .read(&checkout.join("tools").join("container").join("image-version"))
            .map(|s| s.trim().to_owned())
            .unwrap_or_default();
        let image = format!("rivet-card-build:{version}");
        let have_image = self.host.run(&docker, &["image", "inspect", "--format", "{{.Id}}", &image], None).is_ok();
        if have_image {
            out.push(ok(format!("build image {image}")));
        } else {
            out.push(
                Finding::new(Status::Warn, format!("build image {image} is not built yet"))
                    .note("The first tools/container/* run builds it, in minutes."),
            );
        }

        let build_buildroot = "tools/container/build-buildroot.sh --fetch   (takes hours)";
        if self.host.run(&docker, &["volume", "inspect", &br_volume], None).is_err() {
            out.push(
                Finding::new(Status::Fail, format!("no Buildroot volume ({br_volume})"))
                    .note("It holds the kernel, the cross toolchain and the C programs a card carries.")
                    .fix(build_buildroot),
            );
            return out;
        }
        if !have_image {
            out.push(
                Finding::new(Status::Warn, format!("Buildroot volume {br_volume} not checked"))
                    .note("Its contents are read through the build image, which is not built yet."),
            );
            return out;
        }
        let probe = "for f in host/bin/aarch64-linux-gcc images/Image images/bcm2711-rpi-4-b.dtb; do \
                     [ -e /br/$f ] || echo $f; done";
        let volume = format!("{br_volume}:/br:ro");
        let platform = self.host.env("RIVET_DOCKER_PLATFORM").unwrap_or_else(|| "linux/amd64".to_owned());
        let read = self.host.run(
            &docker,
            &["run", "--rm", "--platform", &platform, "-v", &volume, &image, "sh", "-c", probe],
            None,
        );
        out.push(match read {
            Ok(missing) if missing.trim().is_empty() => ok(format!("Buildroot volume {br_volume}")),
            Ok(missing) => Finding::new(Status::Fail, format!("Buildroot volume {br_volume} is incomplete"))
                .note(format!("Missing {}.", missing.split_whitespace().collect::<Vec<_>>().join(", ")))
                .fix(build_buildroot),
            Err(e) => Finding::new(Status::Fail, format!("Buildroot volume {br_volume} could not be read"))
                .why(format!("Through {image}: {e}")),
        });
        out
    }
}

/// Words to lines no wider than [`WIDTH`], each starting with `indent`.
fn wrap(text: &str, indent: &str) -> String {
    let mut out = String::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && indent.len() + line.len() + 1 + word.len() > WIDTH {
            out.push_str(&format!("{indent}{line}\n"));
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push_str(&format!("{indent}{line}\n"));
    }
    out
}

/// The report as text, and how many findings must be fixed.
pub fn render(host: &dyn Host, report: &[(Goal, Vec<Finding>)], this_build: &str, verbose: bool) -> (String, usize) {
    let os = match host.os() {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    };
    let mut text = format!("Rivet doctor · cargo-rivet {this_build} · {os} {}\n\n", host.arch());
    let mut failed = 0;

    let width = report.iter().map(|(g, _)| g.title().len()).max().unwrap_or(0);
    for (goal, findings) in report {
        let bad = findings.iter().filter(|f| f.status == Status::Fail).count();
        let caveats = findings.iter().filter(|f| f.status == Status::Warn).count();
        failed += bad;
        let (mark, verdict) = match (bad, caveats) {
            (0, 0) => ("✓", "ready".to_owned()),
            (0, n) => ("✓", format!("ready, {n} to note")),
            (n, _) => ("✗", format!("{n} to fix")),
        };
        text.push_str(&format!("  {mark} {:<width$}   {verdict}\n", goal.title()));
    }

    for (goal, findings) in report {
        text.push_str(&format!("\n{}  (--for {})\n", goal.title(), goal.word()));
        for f in findings {
            text.push_str(&format!("  {} {}\n", f.status.mark(), f.what));
            let quiet = f.status == Status::Ok || f.status == Status::Skip;
            if !quiet {
                if let Some(note) = &f.note {
                    text.push_str(&wrap(note, "      "));
                }
                for fix in &f.fixes {
                    text.push_str(&format!("      → {fix}\n"));
                }
            }
            if verbose && !f.why.is_empty() {
                let why = if f.status == Status::Ok { f.why.clone() } else { format!("why: {}", f.why) };
                text.push_str(&wrap(&why, "        "));
            }
        }
    }

    text.push('\n');
    if failed == 0 {
        text.push_str("Nothing to fix.");
    } else {
        text.push_str(&format!("{failed} to fix."));
    }
    if !verbose {
        text.push_str(" `cargo rivet doctor --verbose` says why each one matters.");
    }
    text.push('\n');
    (text, failed)
}

/// Prints the report to stdout and, when anything must be fixed, fails with an
/// EMPTY message — `main` exits 1 and prints nothing more, because the report's
/// own last line already says how many things are to fix. A pipe receives the
/// report either way.
///
/// # Errors
///
/// Anything a reported goal requires is missing.
pub fn run(args: &DoctorArgs) -> Result<String, String> {
    let host = RealHost;
    let this_build = crate::build_version();
    let at = credentials::root();
    let report = evaluate(&host, &args.goals, at.as_deref(), &Overrides::from_environment(), &this_build);
    let (text, failed) = render(&host, &report, &this_build, args.verbose);
    if failed == 0 {
        return Ok(text);
    }
    print!("{text}");
    Err(String::new())
}

/// The machine this process is running on.
pub struct RealHost;

impl Host for RealHost {
    fn os(&self) -> &str {
        std::env::consts::OS
    }

    fn arch(&self) -> &str {
        std::env::consts::ARCH
    }

    fn env(&self, name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|v| !v.is_empty())
    }

    fn which(&self, program: &str) -> Option<PathBuf> {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .flat_map(|dir| [dir.join(program), dir.join(format!("{program}.exe"))])
            .find(|candidate| candidate.is_file())
    }

    fn run(&self, program: &str, args: &[&str], cwd: Option<&Path>) -> Result<String, String> {
        let mut command = std::process::Command::new(program);
        command.args(args);
        if let Some(dir) = cwd {
            command.current_dir(dir);
        }
        let out = command.output().map_err(|e| format!("{program} would not start: {e}"))?;
        if out.status.success() {
            return Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned());
        }
        let said = String::from_utf8_lossy(&out.stderr);
        Err(said
            .lines()
            .map(str::trim)
            .rfind(|l| !l.is_empty())
            .map_or_else(|| format!("{program} exited {}", out.status), str::to_owned))
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn read(&self, path: &Path) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    fn cwd(&self) -> Option<PathBuf> {
        std::env::current_dir().ok()
    }

    fn provision_sibling(&self) -> Option<PathBuf> {
        crate::provision::sibling_binary("rivet-provision").ok().filter(|p| p.is_file())
    }

    fn this_binary(&self) -> Option<PathBuf> {
        std::env::current_exe().ok()
    }

    fn cargo_rivet_on_path(&self) -> Option<PathBuf> {
        let path = std::env::var("PATH").ok()?;
        let separator = if cfg!(windows) { ';' } else { ':' };
        crate::first_on_path(&path, separator, &|p: &Path| p.is_file())
    }

    fn same_file(&self, a: &Path, b: &Path) -> bool {
        crate::same_file(a, b)
    }

    fn seed_agrees(&self, seed: &Path, published: &Path) -> Result<bool, String> {
        credentials::agrees(seed, published)
    }
}

#[cfg(test)]
#[path = "doctor_tests.rs"]
mod tests;
