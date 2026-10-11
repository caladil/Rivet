//! **Open a class-matched addon, agree a version, call it, and say what
//! happened** — in the journal, because that is where a proof on a headless
//! board has to end up.
//!
//! Every line it writes begins `addon-probe-host:` so that one `get log`
//! reads the whole article.
//!
//! # What it does, and why in this order
//!
//! 1. **Reports its own `LD_LIBRARY_PATH` and `RIVET_ADDONS`.** If the
//!    supervisor's class match did not happen, everything after this fails for
//!    a reason that has nothing to do with the boundary, and these two lines
//!    are what tells them apart. The second is also the whole of the
//!    enumeration channel as it arrives on a board, printed verbatim.
//! 2. **Checks the trial note** — `.rulings/` R-198 D3. If a previous start
//!    died while loading this class, it is skipped and named, which is the
//!    whole mechanism and costs one `if`.
//! 3. **Enumerates every library of its class**, by name.
//!    `PLAN-Addon-Feature-Set-2026-09-19` T2: a host learns *which* objects it
//!    was joined to, and **two libraries of one class is the case this exists
//!    for**, so a run over one addon proves only half of it.
//! 4. **Asks for the class the old way too**, and reports what came back. With
//!    one library installed that is an open; with two it is a refusal naming
//!    both, which is `.rulings/` R-200 from the outside — a class is a role,
//!    and a host does not silently pick one of two things installed for one
//!    job.
//! 5. **Runs the article against EVERY enumerated object**: describe,
//!    configure, poll twice, then ask it to panic. Twice for the polls because
//!    the second is what shows state survived the first, and because the
//!    cadence the addon asked for is a different number once it is configured.
//!    The panic is the one rule of the ABI that cannot be proven by a passing
//!    test in the same process.
//! 6. **Idles.** `restart = "never"`, so finishing would leave a service in
//!    `done`; idling leaves the journal readable and the process alive for
//!    `get process` to show.

use std::collections::BTreeMap;
use std::path::PathBuf;

use rivetos_addon::host::{CallFailure, Loaded};
use rivetos_addon::{Request, Response};

/// The class this probe accepts, the same string its manifest declares.
const CLASS: &str = "probe-source";

fn main() {
    let data_dir = std::env::var("RIVET_APP_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("addon-probe-host"));

    say(&format!(
        "starting. LD_LIBRARY_PATH={}",
        std::env::var("LD_LIBRARY_PATH").unwrap_or_else(|_| "(unset)".to_string())
    ));
    say(&format!(
        "RIVET_ADDONS={}",
        std::env::var(rivetos_addon::host::ADDONS_ENV)
            .unwrap_or_else(|_| "(unset — this machine names no addons)".to_string())
    ));
    say(&format!("data directory {}", data_dir.display()));

    // ---- the enumeration channel, computed BEFORE the trial note is
    // written, because it is what the note's identity is compared against.
    let joined = rivetos_addon::host::installed(CLASS);
    say(&format!(
        "enumerated {} librar{} of class {CLASS}",
        joined.len(),
        if joined.len() == 1 { "y" } else { "ies" }
    ));
    for (n, entry) in joined.iter().enumerate() {
        say(&format!(
            "  [{n}] {} at {}",
            entry.soname(),
            entry.path.display()
        ));
    }
    let identity = match joined.as_slice() {
        [only] => only.identity(),
        _ => rivetos_addon::abi::soname_for_class(CLASS),
    };

    match rivetos_addon::trial::left_behind(&data_dir, CLASS, &identity) {
        rivetos_addon::trial::LeftBehind::Clear => {}
        rivetos_addon::trial::LeftBehind::Same => {
            say(&format!(
                "SKIPPED the addon of class {CLASS} was being loaded when this program last \
                 stopped, so it is not loaded now. Reinstalling it clears that."
            ));
            idle();
        }
        rivetos_addon::trial::LeftBehind::Unidentified => {
            say(&format!(
                "SKIPPED the addon of class {CLASS} was being loaded when this program last \
                 stopped, so it is not loaded now. This note predates identity tracking, so a \
                 reinstall cannot be told apart from the crash that wrote it; delete {} to \
                 clear it.",
                rivetos_addon::trial::note_path(&data_dir, CLASS).display()
            ));
            idle();
        }
        rivetos_addon::trial::LeftBehind::Different => {
            if let Err(e) = rivetos_addon::trial::forgive(&data_dir, CLASS) {
                say(&format!(
                    "REFUSED a stale trial note for {CLASS} names an object other than what is \
                     installed now, and could not be cleared: {e}"
                ));
                idle();
            }
        }
    }

    if let Err(e) = rivetos_addon::trial::begin(&data_dir, CLASS, &identity) {
        say(&format!(
            "REFUSED to load an addon without recording it first: {e}. An unrecorded load \
             cannot be rolled back."
        ));
        idle();
    }

    // **And the one-of-a-class question, asked the way every host asks it.**
    // Reported whether it succeeds or refuses, because the refusal is half of
    // what this run has to show: two libraries of one class named, and neither
    // of them chosen.
    match Loaded::open(CLASS) {
        Ok(a) => say(&format!(
            "open({CLASS}) gave {} at {}",
            a.soname(),
            a.object().display()
        )),
        Err(e) => say(&format!("open({CLASS}) said: {e}")),
    }

    if joined.is_empty() {
        // Either nothing of this class is installed, or this machine predates
        // the channel — `Loaded::open`'s own answer above says which, and it
        // has already been printed.
        say("no addon of this class is named, so there is nothing to call");
        let _ = rivetos_addon::trial::settled(&data_dir, CLASS);
        idle();
    }

    // The trial note covers the class rather than an object, so it is cleared
    // once, after the first addon has answered — see `article`'s own comment.
    let mut settled = false;
    for entry in &joined {
        article(entry, &data_dir, &mut settled);
    }

    say("ARTICLE COMPLETE");
    idle();
}

/// **The whole conversation with one object.** Called once per enumerated
/// library, so that two libraries of one class are two articles rather than
/// one addon opened twice.
fn article(entry: &rivetos_addon::host::Installed, data_dir: &std::path::Path, settled: &mut bool) {
    let name = entry.soname().to_string();
    say(&format!("---- {name} — {}", entry.path.display()));

    let addon = match Loaded::open_path(&entry.path) {
        Ok(a) => a,
        Err(e) => {
            say(&format!("{name}: {e}"));
            return;
        }
    };

    match addon.call(&Request::Describe) {
        Ok(Response::Described(d)) => {
            say(&format!(
                "{name}: described: {} {} class={} fields={} material={}",
                d.name,
                d.version,
                d.class,
                d.fields.len(),
                d.material.len()
            ));
            if d.class != CLASS {
                say(&format!(
                    "{name}: REFUSED it was named as {CLASS} and calls itself {}; a mislabelled \
                     build is not trusted",
                    d.class
                ));
                return;
            }
        }
        Ok(other) => say(&format!("{name}: describe answered {other:?}")),
        Err(e) => say(&format!("{name}: describe failed: {e}")),
    }

    // An addon has answered once, so the load has settled: from here a crash
    // is the host's to own rather than a reason to skip this class next time.
    // The note is per-class, so it is cleared once however many objects the
    // class turned out to have.
    if !*settled {
        *settled = true;
        match rivetos_addon::trial::settled(data_dir, CLASS) {
            Ok(()) => say("the load settled; the trial note is cleared"),
            Err(e) => say(&format!("the trial note could not be cleared: {e}")),
        }
    }

    poll_once(&addon, &format!("{name}: before configuring"));

    let mut settings = serde_json::Map::new();
    settings.insert("step".to_string(), serde_json::json!(2.5));
    match addon.call(&Request::Configure {
        settings,
        material: BTreeMap::new(),
    }) {
        Ok(Response::Configured { usable, complaint }) => say(&format!(
            "{name}: configured: usable={usable}{}",
            if complaint.is_empty() {
                String::new()
            } else {
                format!(" — {complaint}")
            }
        )),
        Ok(other) => say(&format!("{name}: configure answered {other:?}")),
        Err(e) => say(&format!("{name}: configure failed: {e}")),
    }

    poll_once(&addon, &format!("{name}: first configured poll"));
    poll_once(
        &addon,
        &format!("{name}: second configured poll — the count must have moved"),
    );

    // Rule 3 of the ABI, from the outside: ask for a panic and stay alive.
    let mut breaking = serde_json::Map::new();
    breaking.insert("panic-now".to_string(), serde_json::json!(true));
    let _ = addon.call(&Request::Configure {
        settings: breaking,
        material: BTreeMap::new(),
    });
    match addon.call(&Request::Poll { now_ms: 0 }) {
        Ok(Response::Fault(f)) => say(&format!(
            "{name}: the addon panicked and this program did not: kind={} — {}",
            f.kind, f.sentence
        )),
        Ok(other) => say(&format!("{name}: expected a fault, read {other:?}")),
        Err(e @ CallFailure::Refused(_)) => {
            say(&format!("{name}: the addon refused the call: {e}"));
        }
        Err(e) => say(&format!("{name}: the panicking call failed: {e}")),
    }
}

fn poll_once(addon: &Loaded, what: &str) {
    match addon.call(&Request::Poll { now_ms: 1_789_000_000_000 }) {
        Ok(Response::Polled(p)) => say(&format!(
            "poll ({what}): status={:?} reading={} call_again_in={}ms{}",
            p.status,
            p.reading
                .map_or_else(|| "none".to_string(), |r| format!("{} {}", r.value, r.unit)),
            p.call_again_in_ms,
            if p.complaint.is_empty() {
                String::new()
            } else {
                format!(" — {}", p.complaint)
            }
        )),
        Ok(other) => say(&format!("poll ({what}) answered {other:?}")),
        Err(e) => say(&format!("poll ({what}) failed: {e}")),
    }
}

/// One line, prefixed, flushed. The supervisor reads this program's stdout
/// into the journal (`ADR-022`), so a `println!` is the journal.
fn say(line: &str) {
    println!("addon-probe-host: {line}");
    use std::io::Write as _;
    let _ = std::io::stdout().flush();
}

/// Stay alive so the report stands still and `get process` has something to
/// show. `restart = "never"` means exiting would be final anyway; idling is
/// the more readable of the two.
fn idle() -> ! {
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
