//! Tests for `ADR-056` §3's page.
//!
//! **None of these read `/proc`.** Every parser here takes text, so the whole
//! of the page's judgement is exercised on a build machine with no supervisor
//! anywhere near it — which is the property that lets the wedged cases be
//! tested at all, since a wedged supervisor is precisely what cannot be
//! arranged on demand.

use super::*;

/// A healthy supervisor's `status`, read off `rivet-01` on 2026-09-16.
fn healthy_status(voluntary: u64) -> String {

    format!(
        "Name:\trivet-sysd\n\
         Umask:\t0022\n\
         State:\tS (sleeping)\n\
         Tgid:\t147\n\
         Pid:\t147\n\
         PPid:\t1\n\
         Threads:\t1\n\
         SigBlk:\t0000000000014002\n\
         ShdPnd:\t0000000000000000\n\
         voluntary_ctxt_switches:\t{voluntary}\n\
         nonvoluntary_ctxt_switches:\t12\n"
    )
}

#[test]
fn the_fields_the_page_needs_are_read_out_of_status() {
    let mut s = Sample::default();
    parse_status(&healthy_status(48120), &mut s);
    assert_eq!(s.state, "S (sleeping)");
    assert_eq!(s.threads, 1);
    assert_eq!(s.voluntary, 48120);
    assert_eq!(s.nonvoluntary, 12);
    assert_eq!(s.pending, "0000000000000000");
    assert_eq!(s.blocked, "0000000000014002");
}

/// **A missing line leaves a default and does not fail the read.**
///
/// A page that refused to render because one kernel did not carry one field
/// would be a page that says nothing on the day it is needed.
#[test]
fn a_status_missing_a_field_still_yields_the_rest() {
    let mut s = Sample::default();
    parse_status("State:\tD (disk sleep)\nThreads:\t1\n", &mut s);
    assert_eq!(s.state, "D (disk sleep)");
    assert_eq!(s.threads, 1);
    assert_eq!(s.voluntary, 0);
    assert_eq!(s.pending, "");
}

/// **The comm field is split off by the last bracket, not by whitespace.**
///
/// A process name may contain spaces and parentheses. A whitespace split of
/// this file is a parser that works until somebody names a program badly, and
/// then silently reports another field's number as the CPU time.
#[test]
fn a_program_whose_name_has_spaces_and_brackets_does_not_shift_every_field() {
    let mut s = Sample::default();
    // Fields: 1 pid, 2 comm, 3 state, then 4..=13, 14 utime, 15 stime,
    // 16..=21, 22 starttime.
    let stat = format!(
        "147 (a program (odd) name) S {} 4100 5200 {} 3210000",
        (4..=13).map(|n| n.to_string()).collect::<Vec<_>>().join(" "),
        (16..=21).map(|n| n.to_string()).collect::<Vec<_>>().join(" "),
    );
    parse_stat(&stat, &mut s);
    assert_eq!(s.utime, 4100, "utime is field 14 and the name did not move it");
    assert_eq!(s.stime, 5200);
    assert_eq!(s.starttime, 3_210_000);
}

#[test]
fn the_first_number_of_uptime_is_the_one_that_matters() {
    assert!((parse_uptime("122917.94 488102.11\n") - 122_917.94).abs() < 0.01);
    assert_eq!(parse_uptime(""), 0.0);
}

// ---------------------------------------------------------------------------
// Identification — ADR-056 §3's table, one test per row.
// ---------------------------------------------------------------------------

#[test]
fn the_parent_that_did_not_change_and_calls_itself_the_supervisor_is_the_supervisor() {
    assert_eq!(
        identify(147, 147, Ok("rivet-sysd\n".to_string())),
        Identified::Supervisor {
            pid: 147,
            comm: "rivet-sysd".to_string()
        }
    );
}

#[test]
fn a_parent_of_pid_one_at_render_means_the_supervisor_ended() {
    assert_eq!(
        identify(147, 1, Ok("rivet-initd\n".to_string())),
        Identified::Reparented { was: 147 }
    );
}

#[test]
fn two_different_parents_neither_of_them_pid_one_is_a_restart_nobody_asked_about() {
    assert_eq!(
        identify(147, 900, Ok("rivet-sysd\n".to_string())),
        Identified::Replaced { was: 147, now: 900 }
    );
}

/// The row that matters more than it looks: a page that fell back to scanning
/// `/proc` for a process named `rivet-sysd` would need the directory grant
/// `rivet_core::confinement` refuses on the record.
#[test]
fn a_process_that_was_never_started_by_a_supervisor_says_so_rather_than_guessing() {
    assert_eq!(
        identify(1, 1, Ok("rivet-initd\n".to_string())),
        Identified::NoSupervisor
    );
}

#[test]
fn a_parent_whose_proc_cannot_be_read_is_reported_as_unreadable_not_as_stopped() {
    assert_eq!(
        identify(147, 147, Err("Permission denied".to_string())),
        Identified::Unreadable {
            pid: 147,
            why: "Permission denied".to_string()
        }
    );
}

// ---------------------------------------------------------------------------
// The verdict.
// ---------------------------------------------------------------------------

/// **Any one of the three moving is enough.** A supervisor that burned CPU
/// without parking is alive and busy; one that parked is alive and idle.
#[test]
fn a_loop_that_moved_any_counter_is_turning() {
    for moved in [
        Moved { voluntary: 1, nonvoluntary: 0, ticks: 0 },
        Moved { voluntary: 0, nonvoluntary: 1, ticks: 0 },
        Moved { voluntary: 0, nonvoluntary: 0, ticks: 1 },
    ] {
        assert!(moved.turned(), "{moved:?} is a supervisor that did something");
    }
    assert!(
        !Moved { voluntary: 0, nonvoluntary: 0, ticks: 0 }.turned(),
        "a whole second in which nothing moved at all is the finding this page exists for"
    );
}

fn reading(who: Identified, moved: Moved, second: Sample) -> Reading {
    Reading {
        who,

        second,
        moved,
    }
}

fn wedged_sample() -> Sample {
    let mut s = Sample::default();
    parse_status(&healthy_status(48120), &mut s);
    s.utime = 8_800;
    s.stime = 3_100;
    s.starttime = 200;
    s.uptime = 122_917.9;
    s
}

#[test]
fn a_stopped_supervisor_is_named_as_stopped_in_the_first_sentence_a_reader_meets() {
    let page = html(&reading(
        Identified::Supervisor { pid: 147, comm: "rivet-sysd".to_string() },
        Moved { voluntary: 0, nonvoluntary: 0, ticks: 0 },
        wedged_sample(),
    ));
    assert!(page.contains("The supervisor has stopped."));
    assert!(page.contains("verdict stopped"));
    assert!(
        page.contains("34.1 hours"),
        "the age is what makes frozen counters mean something; got {page}"
    );
    assert!(
        page.contains("What has stopped is the thing that would notice if it were not."),
        "the machine looking fine from the outside is the whole trap of this failure"
    );
}

#[test]
fn a_turning_supervisor_is_named_as_turning() {
    let page = html(&reading(
        Identified::Supervisor { pid: 147, comm: "rivet-sysd".to_string() },
        Moved { voluntary: 41, nonvoluntary: 2, ticks: 3 },
        wedged_sample(),
    ));
    assert!(page.contains("The supervisor is turning."));
    assert!(page.contains("parked itself 41 times"));
    assert!(
        page.contains("used 0.03 seconds of processor time (3 ticks)"),
        "the clause must end itself: an earlier version rendered \"used no processor time of \n         processor time\" on the first live page, {page}"
    );
}

/// `ADR-056` §7 — the supervisor's own safety argument depends on one thread.
#[test]
fn a_second_thread_is_called_out_rather_than_printed_as_a_number() {
    let mut s = wedged_sample();
    s.threads = 2;
    let page = html(&reading(
        Identified::Supervisor { pid: 147, comm: "rivet-sysd".to_string() },
        Moved { voluntary: 5, nonvoluntary: 0, ticks: 0 },
        s,
    ));
    assert!(page.contains("2 threads"));
    assert!(page.contains("That thread count is itself a finding."));
}

/// `ADR-056` §3: useless as a machine interface, on purpose.
#[test]
fn the_page_is_prose_and_carries_no_machine_readable_shape() {
    let page = html(&reading(
        Identified::Supervisor { pid: 147, comm: "rivet-sysd".to_string() },
        Moved { voluntary: 0, nonvoluntary: 0, ticks: 0 },
        wedged_sample(),
    ));
    assert!(!page.contains("{\""), "no JSON anywhere on this path");
    assert!(
        page.contains("Nothing should be built to read it."),
        "the absence of a contract is stated on the page, not only in the ADR"
    );
    assert!(
        page.contains("cannot say <em>where</em>"),
        "the page must say what it cannot answer; the audit 10.6 is why it cannot"
    );
}

/// `ADR-056` §3: one question, and it refuses the rest. A page that listed
/// services would be the second answer to *what is running* that `ADR-011`
/// names, and the backdoor R-195 D2 forbids.
#[test]
fn the_page_answers_one_question_and_names_no_other_service() {
    let page = html(&reading(
        Identified::Supervisor { pid: 147, comm: "rivet-sysd".to_string() },
        Moved { voluntary: 0, nonvoluntary: 0, ticks: 0 },
        wedged_sample(),
    ));
    for other in ["rivet-mcpd", "aura-renderer", "tempmon", "notes", "rivet-proxyd"] {
        assert!(
            !page.contains(other),
            "{other} appears on a page whose whole design is that it reports one process"
        );
    }
}

#[test]
fn a_reparented_process_reports_the_supervisor_as_gone_and_does_not_sample() {
    let page = html(&reading(
        Identified::Reparented { was: 147 },
        Moved { voluntary: 0, nonvoluntary: 0, ticks: 0 },
        Sample::default(),
    ));
    assert!(page.contains("The supervisor is gone."));
    assert!(page.contains("pid 147"));
    assert!(
        !page.contains("The supervisor has stopped."),
        "gone and stopped are different findings and the page must not blur them"
    );
}

#[test]
fn an_unidentifiable_parent_says_so_instead_of_reporting_a_verdict() {
    let page = html(&reading(
        Identified::NoSupervisor,
        Moved { voluntary: 0, nonvoluntary: 0, ticks: 0 },
        Sample::default(),
    ));
    assert!(page.contains("cannot identify a supervisor"));
    assert!(!page.contains("verdict stopped"));
    assert!(!page.contains("verdict turning"));
}

/// A refused `/proc` is a fact about this build's confinement, not about the
/// supervisor — and saying "stopped" there would be a false alarm of exactly
/// the kind `ADR-056` §3 refuses elsewhere.
#[test]
fn a_confinement_that_was_not_granted_is_not_reported_as_a_wedged_supervisor() {
    let page = html(&reading(
        Identified::Unreadable {
            pid: 147,
            why: "Permission denied (os error 13)".to_string(),
        },
        Moved { voluntary: 0, nonvoluntary: 0, ticks: 0 },
        Sample::default(),
    ));
    assert!(page.contains("cannot read it"));
    assert!(page.contains("Permission denied"));
    assert!(!page.contains("The supervisor has stopped."));
}

#[test]
fn the_idle_time_reads_as_a_person_would_say_it() {
    assert_eq!(since(12.0), "12 seconds");
    assert_eq!(since(600.0), "10 minutes");
    assert_eq!(since(122_717.9), "34.1 hours");
}

#[test]
fn processor_time_keeps_the_raw_count_beside_the_seconds() {
    assert_eq!(ticks(0), "no processor time");
    assert_eq!(ticks(250), "2.50 seconds of processor time (250 ticks)");
    assert_eq!(times(0), "not once");
    assert_eq!(times(1), "once");
    assert_eq!(times(41), "41 times");
}
