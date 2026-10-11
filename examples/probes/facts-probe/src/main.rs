//! **Ask the machine what it is, read the supervisor out of `/proc`, and say
//! what happened** — in the journal, because that is where a proof on a
//! headless board has to end up. `ADR-057` §7.
//!
//! Every line begins `facts-probe:` so that one `get log` reads the whole
//! article.
//!
//! # What it does, and why in this order
//!
//! 1. **Begins the watch before anything else.** `Watch::begin` records the pid
//!    of the process that started this one, and it has to happen before
//!    anything can reparent — which is also the first thing a real dashboard
//!    would do.
//! 2. **Asks the facts call and prints the whole answer**, field for field,
//!    with every age rendered as an age. This is the article: a confined
//!    application, with its own uid, getting an answer on a `0666` datagram
//!    socket in a directory its ruleset grants.
//! 3. **Reads `/proc` and prints every row with how it answered** — answered,
//!    refused, absent or indeterminate. `ADR-057` §5's table is a prediction
//!    from the kernel's documented rule until this runs; these lines are what
//!    turn it into a measurement.
//! 4. **Takes a second sample and compares.** One reading of a counter says
//!    nothing; the whole question is a difference.
//! 5. **Stages the silence.** A datagram socket this program binds and never
//!    reads is a supervisor that is alive, has its socket, and has stopped
//!    turning — the wedge's exact shape. A thread reads `/proc` all the way
//!    through the call, and reports how many times it answered while the call
//!    was getting nothing. **That is the pairing, demonstrated.**
//! 6. **Asks a socket that is not there**, so that the three outcomes are shown
//!    to be distinguishable rather than asserted to be.
//! 7. **Idles.** `restart = "never"`, so finishing would leave a service in
//!    `done`; idling leaves the journal readable and the process alive for
//!    `get process` to show.
//!
//! # What this cannot stage, and it is said here rather than left out
//!
//! **A real wedge.** Making the machine's own supervisor stop answering needs a
//! way to stop the supervisor, and this platform has none an operator can
//! reach: there is no verb that signals a process, and the board this runs on
//! is drawing the maintainer's live wall display. Step 5 reproduces the
//! *shape* — a bound socket whose owner is not reading — and step 2's timing
//! beside it is the contrast that matters: the real supervisor answers in
//! milliseconds, and something that has stopped answers not at all.

use std::time::{Duration, Instant};

use rivet_sdk::facts::{self, NotAnswered};
use rivet_sdk::supervisor::{Identified, Reading, Watch};

fn main() {
    // **First, and before anything can reparent.** A dashboard does the same
    // thing in its own first line.
    let watch = Watch::begin();

    say("starting — the facts call's first article (ADR-057)");
    say(&format!(
        "started by pid {}; scratch directory {}",
        watch.started_by(),
        std::env::temp_dir().display()
    ));

    article_the_facts_call();
    article_the_proc_read(watch);
    article_the_pairing(watch);

    say("done — every line above was read on this machine, by this uid");
    idle();
}

/// **Step 2** — the call itself, timed, and every field printed.
fn article_the_facts_call() {
    say(&format!("ask: {} <- `get facts`", facts::SOCKET_PATH));
    let began = Instant::now();
    let answer = facts::ask();
    let took = began.elapsed();

    match answer {
        Ok(f) => {
            say(&format!(
                "ANSWERED in {:.1} ms, protocol {}",
                took.as_secs_f64() * 1000.0,
                f.protocol
            ));
            say(&format!(
                "  identity: host={} version={} slot={}",
                f.hostname.as_deref().unwrap_or("(none)"),
                f.version.as_deref().unwrap_or("(none)"),
                f.slot.as_deref().unwrap_or("(none)")
            ));
            say(&format!(
                "  machine: phase={} health={} mode={}",
                f.phase,
                f.machine.word(),
                f.mode
            ));
            say(&format!(
                "  view: {:.1} s old (the supervisor's last pass), answer {:.3} s old",
                f.view_age().as_secs_f64(),
                f.age().as_secs_f64()
            ));
            say(&format!(
                "  services: {} reported, {} did not fit",
                f.services.len(),
                f.truncated
            ));
            for s in &f.services {
                // **The age is computed HERE and not read off the wire**, which
                // is the whole feature: `health_age` adds this program's own
                // elapsed time to the age the supervisor measured.
                let age = f.health_age(s).map_or_else(
                    || "never answered".to_string(),
                    |d| format!("{:.1} s ago", d.as_secs_f64()),
                );
                say(&format!(
                    "    {:<24} {:<8} enabled={} pid={} restarts={} ready={} health={} ({age})",
                    s.name,
                    s.state,
                    yes_no(s.enabled),
                    s.pid.map_or_else(|| "-".to_string(), |p| p.to_string()),
                    s.restarts,
                    yes_no(s.ready),
                    s.health.word()
                ));
            }
        }
        // **A `Silent` here would be the finding of the run**, not a failure of
        // it: it would mean the supervisor of the machine this is running on
        // has stopped, which is what the surface exists to be able to say.
        Err(why) => say(&format!("NO ANSWER: {why}")),
    }
}

/// **Steps 3 and 4** — the `/proc` read, and the difference between two of
/// them.
fn article_the_proc_read(watch: Watch) {
    let first = watch.read();
    say(&format!(
        "proc: identified {}",
        match first.identified {
            Identified::Supervisor(pid) => format!("the supervisor, pid {pid}"),
            Identified::Reparented =>
                "REPARENTED — the supervisor that started this is gone and pid 1 has us".to_string(),
            Identified::Replaced { was, now } =>
                format!("REPLACED — started by {was}, now under {now}: a restart nobody asked about"),
            Identified::NeverSupervised =>
                "NOT SUPERVISED — pid 1 started this, so there is no supervisor to identify"
                    .to_string(),
        }
    ));
    say(&format!(
        "  window: {}",
        if first.anything_was_read() {
            "granted — this uid can read the supervisor's own /proc"
        } else {
            "REFUSED — nothing was read, so nothing may be concluded about the supervisor"
        }
    ));
    say(&format!("  comm          {}", first.comm.say()));
    say(&format!(
        "  state         {}",
        match &first.state {
            Reading::Answered(c) => c.to_string(),
            other => other.say(),
        }
    ));
    say(&format!("  threads       {}", first.threads.say()));
    say(&format!("  voluntary     {}", first.voluntary.say()));
    say(&format!("  nonvoluntary  {}", first.nonvoluntary.say()));
    say(&format!("  cpu ticks     {}", first.cpu_ticks.say()));
    say(&format!("  uptime ms     {}", first.uptime_ms.say()));
    // **The two rows `ADR-057` §5 predicts and this run measures.**
    say(&format!("  wchan         {}", first.wchan.say()));
    say(&format!("  syscall       {}", first.syscall.say()));

    // One sample of a counter says nothing. `ADR-056` §3 asks for two about a
    // second apart, and the difference is the answer.
    std::thread::sleep(Duration::from_millis(1_200));
    let second = watch.read();
    say(&format!(
        "  turning?      {}",
        match second.moved_since(&first) {
            Some(true) => "YES — a counter advanced between two samples 1.2 s apart",
            Some(false) =>
                "NO — nothing advanced in 1.2 s, which against a climbing uptime is a stopped \
                 supervisor",
            None => "cannot say — neither sample read a counter",
        }
    ));

    // **A third sample, taken once the supervisor has gone back to sleep**, and
    // it exists for one row.
    //
    // The first two are taken moments after this program was spawned, so the
    // supervisor is still `R` — and `0` in `wchan` is the honest answer for a
    // task that is running. To show that `0` is a REFUSAL rather than a
    // reading, the state letter and the symbol have to be seen together on a
    // supervisor that is parked. Compare this line with `get process` over the
    // control plane, which reads the same file as root at the same moment.
    std::thread::sleep(Duration::from_millis(2_500));
    let parked = watch.read();
    say(&format!(
        "  parked        state {} / wchan {}",
        match &parked.state {
            Reading::Answered(c) => c.to_string(),
            other => other.say(),
        },
        match &parked.wchan {
            Reading::Answered(symbol) => symbol.clone(),
            Reading::Indeterminate(_) => "`0` — no symbol for this uid".to_string(),
            other => other.say(),
        }
    ));
}

/// **Steps 5 and 6** — the pairing, and the three outcomes told apart.
fn article_the_pairing(watch: Watch) {
    // A socket this program binds and never reads. That is a supervisor which
    // is alive, holds its socket, and has stopped turning — the wedge's shape,
    // reproduced without touching the machine's own supervisor.
    let staged = std::env::temp_dir().join("staged-wedge.sock");
    let _ = std::fs::remove_file(&staged);
    let Ok(_never_read) = std::os::unix::net::UnixDatagram::bind(&staged) else {
        say("PAIRING NOT SHOWN: this application could not bind a socket of its own in its \
             scratch directory, so there is nothing to stage the silence with");
        return;
    };
    say(&format!(
        "pairing: asking {} — a socket that is bound and never read",
        staged.display()
    ));

    // **`/proc` is read THROUGHOUT the silent call**, on a thread, so the claim
    // is that the two happen at once rather than one after the other.
    let (tell, heard) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut answered = 0_u32;
        let mut attempts = 0_u32;
        let mut last = String::new();
        while heard.try_recv().is_err() {
            let sample = watch.read();
            attempts += 1;
            if sample.anything_was_read() {
                answered += 1;
                last = match &sample.state {
                    Reading::Answered(c) => format!("state {c}"),
                    other => other.say(),
                };
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        (answered, attempts, last)
    });

    let began = Instant::now();
    let answer = facts::ask_at(&staged.to_string_lossy());
    let took = began.elapsed();
    let _ = tell.send(());
    let (answered, attempts, last) = reader.join().unwrap_or((0, 0, String::new()));

    match answer {
        Err(NotAnswered::Silent) => say(&format!(
            "  SILENT after {:.2} s, which is the bound rivet_sdk::facts::PATIENCE sets",
            took.as_secs_f64()
        )),
        other => say(&format!(
            "  UNEXPECTED: a socket nobody reads answered {other:?} after {:.2} s",
            took.as_secs_f64()
        )),
    }
    say(&format!(
        "  and while it was silent, /proc answered {answered} of {attempts} reads — last: {last}"
    ));
    say(
        "  THE PAIRING: the call goes quiet and the /proc read keeps working, so a screen says \
         WHAT the supervisor is doing rather than going stale. The silence here is STAGED, not \
         caused: nothing on this platform can stop the real supervisor, so what is shown is the \
         wedge's shape and not the wedge",
    );
    let _ = std::fs::remove_file(&staged);

    // And the third outcome, so that the three are shown to be distinguishable
    // rather than asserted to be. A screen draws them differently.
    let absent = std::env::temp_dir().join("there-is-no-socket-here");
    let _ = std::fs::remove_file(&absent);
    say(&format!(
        "  a socket that is not there: {:?}",
        facts::ask_at(&absent.to_string_lossy())
    ));
}

const fn yes_no(yes: bool) -> &'static str {
    if yes { "yes" } else { "no" }
}

fn say(line: &str) {
    println!("facts-probe: {line}");
    use std::io::Write as _;
    let _ = std::io::stdout().flush();
}

/// Stay alive so the report stands still and `get process` has something to
/// show. `restart = "never"` means exiting would be final anyway; idling is the
/// more readable of the two.
fn idle() -> ! {
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
