//! A database-backed Rivet application. See `Cargo.toml` for why it exists.
//!
//! It writes a real SQLite file to the one directory it may write to, inserts a
//! row on every tick, and reads the count back — so the journal shows the
//! database growing rather than showing that a library linked.
//!
//! # Two rules it now demonstrates rather than breaking
//!
//! **A demo is documentation, and one that models a forbidden pattern teaches
//! it.** Register `R-33`. This program did two things `ADR-036` forbids, and
//! both were invisible because it worked:
//!
//! 1. It hard-coded `/state/app/sqlite-demo/data`. `ADR-036` §3 rows 2–4: an
//!    application is **handed** its paths, and a literal is correct until the
//!    state volume moves (`ADR-031`). `rivet_sdk::app::data()` is the answer.
//! 2. Every database error was a `?` out of `main` under `restart = "always"`.
//!    `ADR-036` §8.5.2 rules the opposite: **an application treats its own
//!    unparseable state as ABSENT** — one path, the cold-start path, never an
//!    exit. A torn write does not cost you the last few seconds; it leaves a
//!    file that will not open, and a program that exits on it is restarted,
//!    fails the same way, and becomes `PLAN.md` §108's restart-loop fault
//!    splash on a machine nobody can reach.
//!
//! **A power cut is a Rivet's ordinary shutdown**, so the second rule is not a
//! corner case for this program in particular: it is the one that keeps a
//! database-backed kiosk coming back on.

use std::path::{Path, PathBuf};

fn main() {
    // **Told, not assumed** — `ADR-036` §3. `NotHanded` means this process was
    // not started by a supervisor, which is a thing to report rather than to
    // guess a path around.
    let dir = match rivet_sdk::app::data() {
        Ok(dir) => dir,
        Err(why) => {
            println!("sqlite-demo: {why}");
            idle("there is no directory this application may write to");
        }
    };
    let path = dir.join("readings.db");
    println!("sqlite-demo: sqlite {} — opening {}", rusqlite::version(), path.display());

    let db = match open(&path) {
        Ok(db) => db,
        Err(why) => {
            println!("sqlite-demo: the database could not be opened even after a cold start: {why}");
            idle("this machine cannot hold a database");
        }
    };

    let mut written: u64 = 0;
    loop {
        // **A failed tick is reported and retried, not fatal.** The database is
        // open and the disk may be momentarily full, or the file may have been
        // interfered with; either way the next tick is a better answer than a
        // restart, because a restart re-runs everything above for a fault that
        // may already be gone.
        match tick(&db) {
            Ok(rows) => {
                written += 1;
                let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                println!("sqlite-demo: {rows} row(s), {bytes} bytes on {}", dir.display());
            }
            Err(why) => println!(
                "sqlite-demo: this tick wrote nothing: {why} — {written} tick(s) have \
                 succeeded, and the next one is tried in 30 seconds"
            ),
        }
        std::thread::sleep(std::time::Duration::from_secs(30));
    }
}

/// Open the database, and **treat one that will not open as absent** —
/// `ADR-036` §8.5.2.
///
/// A power cut caught mid-write leaves a file that is zero bytes, truncated, or
/// carrying a write-ahead log from a version of the schema that no longer
/// exists. None of those is recoverable by this program and all of them are
/// discardable, because every row here was written by this program and exists
/// nowhere it matters. **That last clause is the whole test** (`ADR-036`
/// §8.5.3): an application that holds something it cannot re-fetch from anywhere
/// — §8.5.5's offline queue of accepted work — must not do this, and wants an
/// append-only file with a discardable tail instead.
///
/// The three files are removed together. A `.db` without its `-wal` is a
/// different database from the one that was there, and leaving either behind is
/// a cold start that is not cold.
fn open(path: &Path) -> Result<rusqlite::Connection, String> {
    match prepare(path) {
        Ok(db) => return Ok(db),
        Err(why) => println!(
            "sqlite-demo: {} will not open ({why}). Treating it as ABSENT and starting cold — \
             ADR-036 §8.5.2. Every row in it was written here and is not held anywhere else \
             that matters; a program whose state IS held nowhere else must not do this.",
            path.display()
        ),
    }
    for suffix in ["", "-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", path.display()));
        match std::fs::remove_file(&sidecar) {
            Ok(()) => println!("sqlite-demo: removed {}", sidecar.display()),
            // Absent is the state being aimed at, so it is not a failure.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{} could not be removed: {e}", sidecar.display())),
        }
    }
    prepare(path)
}

/// Open and shape the database. Every way this can fail is a reason to discard
/// the file, which is why it is one function and not three.
fn prepare(path: &Path) -> Result<rusqlite::Connection, String> {
    let db = rusqlite::Connection::open(path).map_err(|e| e.to_string())?;

    // **Write-ahead logging, on purpose.** A Pi has no battery and gets its
    // power pulled (`ADR-015` §7's own argument for the atomic install), and
    // WAL is what makes an interrupted write leave a readable database rather
    // than a truncated one. It also needs `mmap` and file locking to work on
    // the target, which is exactly the sort of thing that compiles cleanly and
    // fails at run time — so asking for it here is part of the proof.
    //
    // It reduces how often the cold-start path above is taken. It does not
    // remove the need for it: a full `/state`, a half-finished write by an
    // earlier version, and a schema nobody migrated are all outside what a
    // journal mode can promise.
    let mode: String = db
        .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    println!("sqlite-demo: journal_mode = {mode}");

    db.execute(
        "CREATE TABLE IF NOT EXISTS reading (
             id      INTEGER PRIMARY KEY,
             at_unix INTEGER NOT NULL,
             note    TEXT NOT NULL
         )",
        [],
    )
    .map_err(|e| e.to_string())?;
    Ok(db)
}

/// One row in, and the count back out.
fn tick(db: &rusqlite::Connection) -> Result<i64, String> {
    // A Pi has no real-time clock (`ADR-021`), so this is whatever the
    // machine currently believes. It is a value to store, not a claim.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    db.execute(
        "INSERT INTO reading (at_unix, note) VALUES (?1, ?2)",
        rusqlite::params![now, "a row written by a bundled SQLite on a Rivet"],
    )
    .map_err(|e| e.to_string())?;
    db.query_row("SELECT count(*) FROM reading", [], |row| row.get(0))
        .map_err(|e| e.to_string())
}

/// Report and then stay alive — `kiosk-demo`'s rule, and `ADR-036` §8.5.2's.
///
/// A program that exited here would be restarted by the supervisor for ever and
/// the journal would fill with starts rather than with the reason. Staying up
/// with one clear line is what an operator can act on.
fn idle(why: &str) -> ! {
    println!("sqlite-demo: idle — {why}");
    loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("sqlite-demo-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("make the scratch directory");
        dir
    }

    /// **A database that will not open is treated as ABSENT** — `ADR-036`
    /// §8.5.2, register `R-33`.
    ///
    /// This is the file a power cut leaves, and the behaviour it used to
    /// produce was `?` out of `main`, exit non-zero, and be restarted by a
    /// supervisor that would watch it fail identically for ever. What it must
    /// do instead is start cold, which is what this asserts: the open succeeds
    /// and the table is there afterwards.
    #[test]
    fn a_database_that_will_not_open_is_discarded_and_started_cold() {
        let dir = scratch("corrupt");
        let path = dir.join("readings.db");
        // Not a SQLite file. The header is the first sixteen bytes and this is
        // not it, which is exactly what a truncated write leaves.
        std::fs::write(&path, b"this is not a database, it is the tail of one").expect("write");

        let db = open(&path).expect("a corrupt database is absent, not fatal");
        let rows = tick(&db).expect("the cold-started database takes a row");
        assert_eq!(rows, 1, "a cold start begins at nothing, not at what was there");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The write-ahead log goes with the database it belongs to. A `.db`
    /// reopened beside a stale `-wal` is a different database from the one that
    /// was there, so a cold start that leaves one behind is not cold.
    #[test]
    fn the_write_ahead_log_is_discarded_with_the_database() {
        let dir = scratch("wal");
        let path = dir.join("readings.db");
        std::fs::write(&path, b"not a database").expect("write");
        let wal = dir.join("readings.db-wal");
        std::fs::write(&wal, b"not a write-ahead log either").expect("write");

        let db = open(&path).expect("cold start");
        drop(db);
        // Whatever is there now was written by this run. The bytes above cannot
        // have survived it.
        let left = std::fs::read(&wal).unwrap_or_default();
        assert_ne!(left, b"not a write-ahead log either", "the stale log survived");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A database that opens perfectly well is left exactly as it is. The rule
    /// is about state that cannot be read, and a cold start on state that CAN be
    /// read would throw away every row for nothing.
    #[test]
    fn a_healthy_database_is_never_discarded() {
        let dir = scratch("healthy");
        let path = dir.join("readings.db");
        {
            let db = open(&path).expect("a fresh database");
            tick(&db).expect("one row");
            tick(&db).expect("two rows");
        }
        let db = open(&path).expect("reopen");
        let rows: i64 = db
            .query_row("SELECT count(*) FROM reading", [], |row| row.get(0))
            .expect("count");
        assert_eq!(rows, 2, "reopening a healthy database must not clear it");
        std::fs::remove_dir_all(&dir).ok();
    }
}
