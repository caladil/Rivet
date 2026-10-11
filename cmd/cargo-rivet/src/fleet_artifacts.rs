//! **`cargo rivet artifacts` — the menu over the fleet repository**
//! (`ADR-052` §7, `.rulings/` R-225): list it by kind, choose a product, and
//! delete it, delete one version, hide it or unhide it. `fleet retire`,
//! `fleet hide` and `fleet unhide` are its scripted twin, over the same core
//! (`super::retire`).
//!
//! # The screens
//!
//! 1. **The list**, grouped by kind, newest version named. Hidden products are
//!    folded into one line that opens a list of them.
//! 2. **A product**: its versions, and the actions. A delete is asked again,
//!    `(y/N)`; a hide is not, because it is undone by an unhide.
//! 3. **Back to the list**; `x` leaves.
//!
//! **On the way out**, if anything changed: push and sync to the fleet's
//! server `(Y/n)`, then the exact list the server's prune would delete, then
//! the prune as its own step `(y/N)`. Declining leaves it for `cargo rivet
//! fleet sync <server> --prune confirm`.
//!
//! # House style
//!
//! Numbered choices and `x` on every menu (`.rulings/` R-219). The list takes
//! the first entry on a bare Return, as `crate::write_image::choose_with`
//! does; **an action menu takes nothing on a bare Return**, because its
//! entries change the repository. Refusals from the core are shown as they
//! came. Without a terminal it refuses and names the scripted twin, as `cargo
//! rivet deploy` does.

use std::collections::BTreeMap;
use std::io::{IsTerminal, Write};
use std::path::Path;

use rivet_core::artifact::newest_cmp;
use rivet_core::package::port::PublicKey;

use super::Entry;
use super::retire::{self, FleetServer};

/// **Where the interview reads and writes** — a real terminal on a real run,
/// a script in the tests.
pub(super) trait Console {
    /// Show `text` as it is.
    fn say(&mut self, text: &str);

    /// Show `prompt` and read one line, without its line ending.
    ///
    /// # Errors
    ///
    /// Nothing more can be read.
    fn ask(&mut self, prompt: &str) -> Result<String, String>;
}

/// This process's own stdin and stdout.
struct Terminal;

impl Console for Terminal {
    fn say(&mut self, text: &str) {
        print!("{text}");
        std::io::stdout().flush().ok();
    }

    fn ask(&mut self, prompt: &str) -> Result<String, String> {
        print!("{prompt}");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        let read = std::io::stdin().read_line(&mut line).map_err(|e| format!("cannot read an answer: {e}"))?;
        if read == 0 {
            return Err("stdin closed before an answer was given".to_owned());
        }
        Ok(line.trim_end_matches(['\r', '\n']).to_owned())
    }
}

/// `cargo rivet artifacts --help`.
#[must_use]
pub fn usage() -> String {
    "usage: cargo rivet artifacts\n\
     \n\
     THE MENU OVER THE FLEET REPOSITORY (ADR-052 §7), for a person at a\n\
     terminal. It takes NO ARGUMENTS. It lists ~/.rivet/fleet by kind, with\n\
     hidden products folded into one line that opens; a product is chosen,\n\
     then an action: delete the whole product, delete one version, hide it\n\
     from every fleet, or unhide it. A delete is refused for an application a\n\
     machine's card carries or pins, naming the machines and how to unpin.\n\
     \n\
     On the way out, if anything changed, it offers to push and sync to the\n\
     fleet's server (Y/n), then shows the exact list the server's prune would\n\
     delete and offers the prune as its own step (y/N).\n\
     \n\
     AN AGENT OR A SCRIPT USES THE SCRIPTED TWIN, over the same core:\n\
     \x20 cargo rivet fleet retire <product> [--kind K] [--version V] [--sync [confirm]]\n\
     \x20 cargo rivet fleet hide   <product> [--kind K] [--sync [confirm]]\n\
     \x20 cargo rivet fleet unhide <product> [--kind K] [--sync [confirm]]\n\
     `cargo rivet fleet --help` says what each does.\n"
        .to_owned()
}

/// **`cargo rivet artifacts <anything>` is refused**, as `deploy`'s is: the
/// menu takes no arguments, and one given would have been ignored.
#[must_use]
pub fn refuses(word: &str) -> String {
    format!(
        "`cargo rivet artifacts` takes no arguments, and `{word}` would have been IGNORED. NOTHING \
         WAS DONE.\n\
         \x20 a person              `cargo rivet artifacts`, alone, at a terminal\n\
         \x20 an agent or a script  `cargo rivet fleet retire | hide | unhide <product> …` (fleet --help)"
    )
}

fn refusal_for_a_scripted_caller() -> String {
    "this run has no terminal, and `cargo rivet artifacts` is a menu — a human tool. A script or an \
     agent uses its scripted twin, over the same core:\n\
     \x20 cargo rivet fleet list [--hidden]\n\
     \x20 cargo rivet fleet retire <product> [--kind K] [--version V] [--sync [confirm]]\n\
     \x20 cargo rivet fleet hide <product> [--kind K] [--sync [confirm]]\n\
     \x20 cargo rivet fleet unhide <product> [--kind K] [--sync [confirm]]\n\
     Nothing was read and nothing was changed."
        .to_owned()
}

/// `cargo rivet artifacts` — the whole interview, start to `x`.
///
/// # Errors
///
/// No terminal; the fleet repository or the register cannot be read; or the
/// terminal closed mid-answer.
pub fn run() -> Result<String, String> {
    if !std::io::stdin().is_terminal() {
        return Err(refusal_for_a_scripted_caller());
    }
    let root = super::deploy_root(None)?;
    let register = super::load_register()?;
    interview(
        &mut Terminal,
        &root,
        &register,
        &mut super::application_key,
        &mut || retire::connect(&rivet_remote::endpoint::Endpoint::default(), "cargo rivet artifacts"),
    )
}

/// **The interview** over the fleet repository at `root` — every input
/// handed in, so a test drives it with a script, a scratch register and a
/// fake server.
///
/// `connect` is asked only on the way out, and only if something changed.
///
/// # Errors
///
/// `root` is not a fleet repository, its manifest does not read, or the
/// console closed. A refusal from the core is shown and is not an error.
pub(super) fn interview(
    console: &mut dyn Console,
    root: &Path,
    register: &rivetos_register::Register,
    apk_key: &mut dyn FnMut() -> Result<PublicKey, String>,
    connect: &mut dyn FnMut() -> Result<Box<dyn FleetServer>, String>,
) -> Result<String, String> {
    super::require_repository(root)?;
    let mut changed = false;
    loop {
        let chosen = match list_screen(console, root)? {
            Listed::Exit => break,
            Listed::Product(product, kind) => Some((product, kind)),
            Listed::Hidden => hidden_screen(console, root)?,
        };
        if let Some((product, kind)) = chosen {
            changed |= product_screen(console, root, &product, &kind, register, apk_key)?;
        }
    }
    if !changed {
        return Ok("\nExited. Nothing was changed.\n".to_owned());
    }
    on_the_way_out(console, root, connect)?;
    Ok("\nExited.\n".to_owned())
}

// ---------------------------------------------------------------------------
// reading an answer
// ---------------------------------------------------------------------------

/// A numbered menu's answer: `Some(index)`, or `None` for `x`. A bare Return
/// takes `default`, or is asked again when there is none; anything else out
/// of range is refused and asked again.
fn choose(console: &mut dyn Console, count: usize, default: Option<usize>) -> Result<Option<usize>, String> {
    let prompt = match default {
        Some(d) => format!("\n  choose 1-{count} [{}], or x: ", d + 1),
        None => format!("\n  choose 1-{count}, or x: "),
    };
    loop {
        let answer = console.ask(&prompt)?;
        let typed = answer.trim();
        if typed.eq_ignore_ascii_case("x") {
            return Ok(None);
        }
        if typed.is_empty() {
            if let Some(d) = default {
                return Ok(Some(d));
            }
            console.say("  nothing was chosen: a number, or x\n");
            continue;
        }
        match typed.parse::<usize>() {
            Ok(n) if (1..=count).contains(&n) => return Ok(Some(n - 1)),
            _ => console.say(&format!("  {typed:?} is not a number between 1 and {count}, or x\n")),
        }
    }
}

/// A `(y/N)` or `(Y/n)` answer: a bare Return takes `default`; otherwise only
/// `y` or `yes` is yes — a mistyped answer is never taken as approval.
fn confirm(console: &mut dyn Console, question: &str, default: bool) -> Result<bool, String> {
    let marks = if default { "(Y/n)" } else { "(y/N)" };
    let answer = console.ask(&format!("\n  {question} {marks}: "))?;
    let typed = answer.trim().to_ascii_lowercase();
    if typed.is_empty() {
        return Ok(default);
    }
    Ok(typed == "y" || typed == "yes")
}

// ---------------------------------------------------------------------------
// the list
// ---------------------------------------------------------------------------

enum Listed {
    Product(String, String),
    Hidden,
    Exit,
}

/// Kinds in the order a person looks for them.
fn kind_rank(kind: &str) -> u8 {
    match kind {
        "application" => 0,
        "library" => 1,
        "apk" => 2,
        "update" => 3,
        "card" => 4,
        _ => 5,
    }
}

/// Products keyed by `(kind rank, kind, product)`, so iterating groups them by
/// kind in [`kind_rank`]'s order and by name within a kind.
type Grouped = BTreeMap<(u8, String, String), Vec<Entry>>;

/// Every `(product, kind)` the manifest holds, grouped by kind, with its
/// entries — `hidden` selects the hidden ones or the rest.
fn products(root: &Path, hidden: bool) -> Result<Grouped, String> {
    let set = super::read_hidden(root)?;
    let mut out = Grouped::new();
    for entry in super::read_manifest(root)? {
        if super::is_hidden(&set, &entry) != hidden {
            continue;
        }
        out.entry((kind_rank(&entry.kind), entry.kind.clone(), entry.product.clone())).or_default().push(entry);
    }
    Ok(out)
}

fn summary(entries: &[Entry]) -> String {
    let newest = entries.iter().max_by(|a, b| newest_cmp(super::build_key(a), super::build_key(b)));
    format!(
        "{} version{}, newest {}",
        entries.len(),
        if entries.len() == 1 { "" } else { "s" },
        newest.map_or("?", |e| e.version.as_str())
    )
}

fn list_screen(console: &mut dyn Console, root: &Path) -> Result<Listed, String> {
    let shown = products(root, false)?;
    let folded = products(root, true)?;
    if shown.is_empty() && folded.is_empty() {
        console.say(&format!(
            "\n  the fleet repository at {} holds nothing yet. `cargo rivet fleet publish` adds one.\n",
            root.display()
        ));
        return Ok(Listed::Exit);
    }
    let mut text = format!("\n  the fleet repository at {}\n", root.display());
    let mut rows: Vec<(String, String)> = Vec::new();
    let mut heading = String::new();
    for ((_, kind, product), entries) in &shown {
        if *kind != heading {
            text.push_str(&format!("\n  {kind}\n"));
            heading.clone_from(kind);
        }
        rows.push((product.clone(), kind.clone()));
        text.push_str(&format!("    {}) {product} — {}\n", rows.len(), summary(entries)));
    }
    let hidden_at = (!folded.is_empty()).then(|| {
        text.push_str(&format!(
            "\n    {}) {} hidden — withheld from every fleet; open them\n",
            rows.len() + 1,
            folded.len()
        ));
        rows.len()
    });
    text.push_str("    x) exit\n");
    console.say(&text);
    let count = rows.len() + usize::from(hidden_at.is_some());
    Ok(match choose(console, count, Some(0))? {
        None => Listed::Exit,
        Some(n) if Some(n) == hidden_at => Listed::Hidden,
        Some(n) => {
            let (product, kind) = rows[n].clone();
            Listed::Product(product, kind)
        }
    })
}

fn hidden_screen(console: &mut dyn Console, root: &Path) -> Result<Option<(String, String)>, String> {
    let folded = products(root, true)?;
    let mut text = "\n  hidden — in this repository, withheld from every fleet\n".to_owned();
    let mut rows: Vec<(String, String)> = Vec::new();
    for ((_, kind, product), entries) in &folded {
        rows.push((product.clone(), kind.clone()));
        text.push_str(&format!("    {}) {product} ({kind}) — {}\n", rows.len(), summary(entries)));
    }
    text.push_str("    x) back to the list\n");
    console.say(&text);
    Ok(choose(console, rows.len(), Some(0))?.map(|n| rows[n].clone()))
}

// ---------------------------------------------------------------------------
// a product
// ---------------------------------------------------------------------------

/// One product's screen, until `x` or until it is gone. Answers whether the
/// repository changed.
fn product_screen(
    console: &mut dyn Console,
    root: &Path,
    product: &str,
    kind: &str,
    register: &rivetos_register::Register,
    apk_key: &mut dyn FnMut() -> Result<PublicKey, String>,
) -> Result<bool, String> {
    let mut changed = false;
    loop {
        let mut entries: Vec<Entry> =
            super::read_manifest(root)?.into_iter().filter(|e| e.product == product && e.kind == kind).collect();
        if entries.is_empty() {
            return Ok(changed);
        }
        entries.sort_by(|a, b| newest_cmp(super::build_key(a), super::build_key(b)).reverse());
        let hidden = super::read_hidden(root)?.iter().any(|h| h.product == product && h.kind == kind);

        let mut text = format!(
            "\n  {product} ({kind}){}\n",
            if hidden { " — HIDDEN, withheld from every fleet" } else { "" }
        );
        for e in &entries {
            text.push_str(&format!(
                "      {:<24} {}{}  added {}  {}\n",
                e.version,
                e.arch,
                e.platform.as_deref().map(|p| format!(" {p}")).unwrap_or_default(),
                e.added,
                &e.digest[..e.digest.len().min(12)],
            ));
        }
        text.push_str(&format!(
            "\n    1) {}\n    2) delete one version\n    3) delete the whole product\n    x) back to the list\n",
            if hidden { "unhide it — offer it to the fleets again" } else { "hide it from every fleet" }
        ));
        console.say(&text);

        match choose(console, 3, None)? {
            None => return Ok(changed),
            Some(0) => {
                let outcome =
                    if hidden { retire::unhide(root, product, Some(kind)) } else { retire::hide(root, product, Some(kind)) };
                changed |= show(console, outcome);
            }
            Some(1) => {
                let mut versions: Vec<&str> = Vec::new();
                for e in &entries {
                    if !versions.contains(&e.version.as_str()) {
                        versions.push(&e.version);
                    }
                }
                let mut menu = "\n  which version\n".to_owned();
                for (n, v) in versions.iter().enumerate() {
                    menu.push_str(&format!("    {}) {v}\n", n + 1));
                }
                menu.push_str("    x) back\n");
                console.say(&menu);
                let Some(n) = choose(console, versions.len(), None)? else {
                    continue;
                };
                let version = versions[n].to_owned();
                if confirm(console, &format!("delete {product} {version} ({kind}) from the fleet repository?"), false)? {
                    changed |= show(console, retire::retire(root, product, Some(kind), Some(&version), register, apk_key));
                } else {
                    console.say("  not deleted\n");
                }
            }
            Some(_) => {
                let question = format!(
                    "delete {product} ({kind}) — all {} entr{} — from the fleet repository?",
                    entries.len(),
                    if entries.len() == 1 { "y" } else { "ies" }
                );
                if confirm(console, &question, false)? {
                    changed |= show(console, retire::retire(root, product, Some(kind), None, register, apk_key));
                } else {
                    console.say("  not deleted\n");
                }
            }
        }
    }
}

/// Show what the core said, as it said it; answer whether it changed anything.
fn show(console: &mut dyn Console, outcome: Result<String, String>) -> bool {
    match outcome {
        Ok(report) => {
            console.say(&format!("\n{report}"));
            !report.contains("Nothing was changed")
        }
        Err(refusal) => {
            console.say(&format!("\n{refusal}\n"));
            false
        }
    }
}

// ---------------------------------------------------------------------------
// on the way out
// ---------------------------------------------------------------------------

/// Push and sync `(Y/n)`, the exact prune list, then the prune `(y/N)` as its
/// own step. Every refusal is shown; none stops the exit.
fn on_the_way_out(
    console: &mut dyn Console,
    root: &Path,
    connect: &mut dyn FnMut() -> Result<Box<dyn FleetServer>, String>,
) -> Result<(), String> {
    console.say("\n  the changes are committed to this workstation's fleet repository.\n");
    let later = "`cargo rivet fleet push <server> confirm`, then `cargo rivet fleet sync <server> --prune confirm`";
    let server = match connect() {
        Ok(server) => server,
        Err(why) => {
            console.say(&format!("\n{why}\n  nothing was sent; {later} reach it later.\n"));
            return Ok(());
        }
    };
    let label = server.label().to_owned();
    if !confirm(console, &format!("push and sync to {label}?"), true)? {
        console.say(&format!("  nothing was sent; {later} reach it later.\n"));
        return Ok(());
    }
    match server.send_and_push(root, true) {
        Ok(report) => console.say(&format!("\n{report}")),
        Err(report) => {
            console.say(&format!("\n{report}\n  the sync did not complete, so no prune is offered.\n"));
            return Ok(());
        }
    }
    let listing = match server.listing() {
        Ok(listing) => listing,
        Err(why) => {
            console.say(&format!("\n{why}\n  no prune is offered.\n"));
            return Ok(());
        }
    };
    console.say(&format!("\n{}", retire::prune_plan(&listing, &label)));
    let offer = matches!(listing.prune(), Ok(remove) if !remove.is_empty());
    if !offer {
        return Ok(());
    }
    if !retire::read_at_local_main(root, &listing) {
        console.say(&format!(
            "  {label}'s text is not this workstation's main, so no prune is offered; {later}.\n"
        ));
        return Ok(());
    }
    if confirm(console, "prune it now?", false)? {
        match server.prune() {
            Ok(body) => console.say(&format!("\n  PRUNED, by {label} against its own manifest:\n{body}")),
            Err(why) => console.say(&format!("\n  PRUNE REFUSED by {label}:\n{why}\n")),
        }
    } else {
        console.say(&format!(
            "  left in {label}'s store; `cargo rivet fleet sync {label} --prune confirm` removes it.\n"
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "fleet_artifacts_tests.rs"]
mod tests;
