//! **Spins one core, then reads its own cgroup's accounting back** —
//! `tools/qemu/adr-007-cpu-proof.sh`'s instrument for `ADR-007` §7.2.5 and
//! `ADR-015` §3.2.3: *"a cap that was written to a file nobody checked is not
//! a cap."*
//!
//! # Why a SYSTEM service slot, and not a signed application
//!
//! An installed application is Landlock-confined (`ADR-042`), and
//! `rivet_core::confinement`'s own module documentation is explicit that
//! `/proc` and `/sys` are granted **nowhere beyond one file** — so a probe
//! packaged as an application could never open its own
//! `/sys/fs/cgroup/rivet/<name>/cpu.stat` to prove anything; the read would
//! fail with `EACCES` regardless of whether the cap enforced.
//!
//! An **unconfined system service** (`ADR-017` §4.1's first origin, without a
//! home) runs as uid 0 with no Landlock policy at all
//! (`crate::service::IMAGE_DEFINED`'s `confinement: None`, mirrored in
//! `rivet-sysd`), and `[services.<name>]`'s `cpu` key caps a system service
//! exactly as it caps an application — the document does not know or care
//! which kind of service a name belongs to. So this binary is installed in
//! place of `rivet-netd` (already `enabled` by default) for the CAPPED half
//! and `rivet-mcpd` (enabled by image default) for the UNCAPPED
//! half, which is `tools/qemu/run.sh`'s own documented use of `--netd` and
//! `--mcpd`: substituting the binary at a path the supervisor already
//! knows how to run. A homed image service is refused `/sys` by its ruleset,
//! so its slot cannot carry this probe.
//!
//! # What it measures, and over what window
//!
//! On start it reads `cpu.max` (so the printed line carries the cap the
//! kernel actually holds, not the one the document asked for — `ADR-024`
//! §8.1's whole lesson is that those can differ) and `cpu.stat`'s
//! `usage_usec`, spins one core with a tight arithmetic loop for
//! [`SPIN_SECS`] of WALL-CLOCK time (`Instant`, never a sleep — a sleeping
//! thread asks the scheduler for nothing and would prove a cap by doing
//! nothing to test it), and reads `usage_usec` again. The difference over the
//! measured wall-clock window is the service's own account of how much CPU it
//! was actually given, which is what `AGENTS.md` §5 calls reading the
//! system's own answer rather than trusting the write that asked for one.
//!
//! It then parks itself rather than exiting, so a restart policy of `always`
//! never respawns it mid-measurement and a policy of `on-failure`
//! (`rivet-netd`'s) never sees an exit to react to.

use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};

/// How long to spin. Long enough that `cpu.max`'s 100000µs period
/// (`rivet_core::service::CpuLimit::PERIOD_US`) has run sixty-odd times, so a
/// cap converges to something close to its steady state rather than being
/// measured mid-burst; short enough that a proof script does not wait
/// minutes for two of these.
const SPIN_SECS: u64 = 6;

/// `/sys/fs/cgroup/rivet` — `cmd/rivet-sysd::kernel::CGROUP_ROOT`, spelled
/// again here rather than imported: this crate is its own workspace root
/// (see `Cargo.toml`) and cannot depend on the binary crate it is proving
/// something about.
const CGROUP_ROOT: &str = "/sys/fs/cgroup/rivet";

fn main() {
    // **`argv[0]`, and not a name this binary is told** — `tools/qemu/
    // testapp`'s own convention: the harness installs one binary at a path
    // the supervisor already spawns (`rivet-netd` or `rivet-mcpd`),
    // and the service name IS that path's last component. Deriving it here
    // is what lets one binary serve both halves of the proof.
    let arg0 = std::env::args().next().unwrap_or_default();
    let name = Path::new(&arg0)
        .file_name()
        .map_or_else(|| "cpu-spin-probe".to_string(), |n| n.to_string_lossy().into_owned());

    println!("cpu-spin-probe: installed as {name}, pid {}", std::process::id());
    let _ = std::io::stdout().flush();

    let cgroup = format!("{CGROUP_ROOT}/{name}");
    // Not `{cpu_max:?}` — the console line is itself re-emitted inside a
    // JSON `msg` field by the supervisor's capture (`ADR-022` §4), and a
    // Debug-quoted string here would put a SECOND, unescaped `"` in the
    // line — `adr-007-cpu-proof.sh` greps up to the first `"`, so a Debug
    // quote truncated this line to nothing useful. Plain text has none.
    let cpu_max = read_trimmed(&format!("{cgroup}/cpu.max")).replace('\n', " ");
    println!("cpu-spin-probe: name={name} cpu_max=[{cpu_max}]");

    let before = read_usage_usec(&cgroup);
    println!("cpu-spin-probe: name={name} usage_usec_before={before:?}");
    let _ = std::io::stdout().flush();

    let start = Instant::now();
    // A tight loop with a volatile-ish side effect (the black-box read
    // prevents the optimiser from proving the loop does nothing and deleting
    // it), checking the clock every million iterations rather than every one
    // — a `clock_gettime` every iteration would itself compete for the CPU
    // this is trying to measure.
    let mut counter: u64 = 0;
    loop {
        for _ in 0..1_000_000 {
            counter = counter.wrapping_add(counter.wrapping_mul(2_654_435_761).rotate_left(13));
        }
        if start.elapsed() >= Duration::from_secs(SPIN_SECS) {
            break;
        }
    }
    let elapsed = start.elapsed();
    // Printed so a reader can see the loop was not optimised away — this
    // value has no other use.
    println!("cpu-spin-probe: name={name} counter={counter}");

    let after = read_usage_usec(&cgroup);
    println!(
        "cpu-spin-probe: name={name} elapsed_us={} usage_usec_after={after:?}",
        elapsed.as_micros()
    );
    if let (Some(b), Some(a)) = (before, after) {
        println!(
            "cpu-spin-probe: RESULT name={name} elapsed_us={} delta_usage_usec={}",
            elapsed.as_micros(),
            a.saturating_sub(b)
        );
    } else {
        println!("cpu-spin-probe: RESULT name={name} REFUSED — could not read {cgroup}/cpu.stat");
    }
    let _ = std::io::stdout().flush();

    // **Park rather than exit.** Both slots are `on-failure`, and a probe that
    // exited non-zero would be respawned and print a second, confusing RESULT
    // line into the same console log. Parking leaves exactly one, whichever
    // restart policy the slot it is installed in carries.
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

fn read_trimmed(path: &str) -> String {
    fs::read_to_string(path).map_or_else(
        |e| format!("<unreadable: {e}>"),
        |s| s.trim().to_string(),
    )
}

/// Parse `usage_usec <N>` out of `cpu.stat`. `None` if the file could not be
/// read or the key is not the one line this cares about — refuse rather than
/// guess (`AGENTS.md` §5), because a probe that defaulted to `0` on a read
/// failure would report a suspiciously perfect cap.
fn read_usage_usec(cgroup_dir: &str) -> Option<u64> {
    let text = fs::read_to_string(format!("{cgroup_dir}/cpu.stat")).ok()?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("usage_usec ") {
            return rest.trim().parse().ok();
        }
    }
    None
}
