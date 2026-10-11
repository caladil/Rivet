//! `rivet get memory` — **what this machine's memory looks like, and whether it
//! is struggling.**
//!
//! `ADR-035` phase 4, and one of the five the maintainer named on 2026-08-29
//! (processes, memory, networking, devices, uptime). Nothing answered it before:
//! every memory reading taken off a board came from `open --raw /proc/meminfo`
//! in a shell that phase 4 exists to remove.
//!
//! # `available` is the answer; `free` is a trap
//!
//! A healthy Linux has almost **no** free memory, because everything spare is
//! page cache. An operator — or an agent — reading `free_bytes` as headroom
//! will diagnose exhaustion on a machine that is working perfectly, and the
//! mistake is old enough to have its own websites about it.
//!
//! So this reports both, and [`crate::operation::machine::MemoryRead`]'s own
//! documentation says which one to read. Dropping `free` entirely was
//! considered and rejected: it is a fact the kernel states, and a surface that
//! hides facts to prevent misreadings is one nobody can check.
//!
//! # Pressure is the part that answers the question people actually ask
//!
//! *"Is this box struggling?"* is not answerable from a free/total ratio.
//! Pressure stall information measures the thing directly — how long tasks
//! spent waiting on memory — and its 60-second average is how a machine that is
//! **fine now but was thrashing a minute ago** gets caught. `docs/design-
//! candidates/Admin-Surface-Survey.md` §7.4 named it as a gap that neither this
//! project nor an external review had listed.
//!
//! **It is absent on a kernel without `CONFIG_PSI`**, and absence is reported as
//! a fact rather than as zeroes — zeroes would say *"nothing is stalling"*,
//! which is a claim this build would not have measured.

use super::machine::{Observed, SlabCache, SlabCaches};
use super::{ErrorCode, Field, OperationError};
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "memory";

/// What this subject's result carries. `ADR-006` §7.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was read. Always `memory` in this build.",
    },
    Field {
        name: "total_bytes",
        description: "Everything the kernel manages. BYTES, not the kibibytes /proc/meminfo \
                      states — the conversion happens once, in the adapter, so that nothing \
                      reading this has to remember a unit.",
    },
    Field {
        name: "available_bytes",
        description: "THE ONE TO READ. The kernel's own estimate of what a new allocation \
                      could get: free memory plus what it would reclaim. Compare this against \
                      `total_bytes` when asking whether a machine has headroom.",
    },
    Field {
        name: "free_bytes",
        description: "Memory that is untouched. NOT HEADROOM, and reading it as headroom is \
                      the classic misdiagnosis of Linux memory: a healthy machine has almost \
                      none, because everything spare is page cache. Reported because it is a \
                      fact the kernel states, not because it answers a useful question.",
    },
    Field {
        name: "cached_bytes",
        description: "Page cache — the reason `free_bytes` and `available_bytes` differ so \
                      widely, and evictable under pressure.",
    },
    Field {
        name: "swap_total_bytes",
        description: "Swap configured. Zero on a stock Rivet, which has none.",
    },
    Field {
        name: "swap_free_bytes",
        description: "Swap unused. Equal to `swap_total_bytes` when nothing has swapped.",
    },
    Field {
        name: "pressure",
        description: "Pressure stall information, or ABSENT on a kernel built without \
                      CONFIG_PSI. Absence is reported as absence rather than as zeroes, \
                      because zeroes would assert that nothing is stalling and this build \
                      would not have measured that. Values are HUNDREDTHS OF A PERCENT: this \
                      surface has no floating-point value, and a whole percent would collapse \
                      the difference between a machine stalling 0.4% of the time and one that \
                      is not stalling at all. `some_*` is at least one task waiting; `full_*` \
                      is EVERY runnable task waiting, which means the machine did no work. The \
                      60-second averages are how a box that is fine NOW but was thrashing a \
                      minute ago is caught — the 10-second ones will already have recovered.",
    },
    Field {
        name: "slab_bytes",
        description: "The kernel's slab caches, reclaimable and not (/proc/meminfo `Slab`). \
                      This and every field after it is the kernel's own holdings — the reading \
                      that attributes a fall in `available_bytes` which no process's resident \
                      size explains. EACH IS ABSENT when the kernel does not write its line, \
                      never zero, because zero would assert a measurement nobody took.",
    },
    Field {
        name: "slab_reclaimable_bytes",
        description: "The part of `slab_bytes` the kernel would give back under pressure \
                      (`SReclaimable`).",
    },
    Field {
        name: "slab_unreclaimable_bytes",
        description: "The part of `slab_bytes` it would not (`SUnreclaim`). A steady rise here \
                      while processes hold flat is the signature of a kernel-side leak.",
    },
    Field {
        name: "shmem_bytes",
        description: "Shared memory and tmpfs (`Shmem`), which includes buffers a compositor \
                      shares with its clients.",
    },
    Field {
        name: "page_tables_bytes",
        description: "Page tables of every address space (`PageTables`).",
    },
    Field {
        name: "kernel_stack_bytes",
        description: "One kernel stack per thread (`KernelStack`), so it follows the thread \
                      count.",
    },
    Field {
        name: "vmalloc_used_bytes",
        description: "The kernel's virtually contiguous allocations (`VmallocUsed`).",
    },
    Field {
        name: "cma_total_bytes",
        description: "The contiguous region reserved for devices such as the GPU (`CmaTotal`). \
                      Absent on a kernel built without CONFIG_CMA.",
    },
    Field {
        name: "cma_free_bytes",
        description: "What of that region is unallocated (`CmaFree`). A fall with nothing new \
                      drawing is display buffers being held.",
    },
    Field {
        name: "slab_cache_count",
        description: "How many slab caches the kernel listed in /proc/slabinfo, every one of \
                      them and not only those in `slab_caches`. ABSENT when the listing could \
                      not be read (it is root's to read) or did not parse.",
    },
    Field {
        name: "slab_caches",
        description: "The slab caches holding the most, largest first, at most twenty: each \
                      `name`, `active_objects`, `objects`, `object_size` and `bytes`, where \
                      `bytes` is `objects` times `object_size` as /proc/slabinfo states them \
                      (short of whole slabs by each slab's unused tail). `merged` names every \
                      OTHER cache the kernel merged into this one, whose objects are counted \
                      here; it is absent when /sys/kernel/slab could not be read. This is the \
                      reading that says WHICH cache a rise in `slab_unreclaimable_bytes` went \
                      to.",
    },
    Field {
        name: "slab_caches_refused",
        description: "Present INSTEAD of the two fields above when /proc/slabinfo was read and \
                      is not in a form this build understands, carrying the text it could not \
                      parse. A listing is refused whole rather than reported with a line \
                      left out, because the line left out could be the one that names the \
                      leak.",
    },
];

/// How many caches `slab_caches` carries. Twenty covers everything holding more
/// than a few pages on a Pi 4, and keeps a capture taken every five minutes
/// small.
pub const SLAB_CACHES_REPORTED: usize = 20;

/// Perform it.
///
/// # Errors
///
/// If a second word was given, or the kernel's own files could not be read.
pub(super) fn perform(
    arguments: &[&str],
    observed: &mut dyn Observed,
) -> Result<Value, OperationError> {
    if let Some(extra) = arguments.get(1) {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            SUBJECT,
            format!(
                "`{SUBJECT}` takes no second word and {extra:?} is one. The whole reading is \
                 the answer; select part of it with a JSON tool rather than with an argument \
                 this build does not have."
            ),
        ));
    }

    let read = observed
        .read_memory()
        .map_err(|e| OperationError::about(ErrorCode::MachineRefused, SUBJECT, e.why().to_string()))?;

    let bytes = |n: u64| Value::Integer(i64::try_from(n).unwrap_or(i64::MAX));
    let mut fields = vec![
        ("subject", Value::str(SUBJECT)),
        ("total_bytes", bytes(read.total_bytes)),
        ("available_bytes", bytes(read.available_bytes)),
        ("free_bytes", bytes(read.free_bytes)),
        ("cached_bytes", bytes(read.cached_bytes)),
        ("swap_total_bytes", bytes(read.swap_total_bytes)),
        ("swap_free_bytes", bytes(read.swap_free_bytes)),
    ];

    // **Present or absent, never zeroed.** A kernel with no PSI has not
    // measured zero stalling; it has measured nothing.
    if let Some(p) = read.pressure {
        fields.push((
            "pressure",
            Value::object(vec![
                ("some_avg10", Value::Integer(i64::from(p.some_avg10))),
                ("some_avg60", Value::Integer(i64::from(p.some_avg60))),
                ("full_avg10", Value::Integer(i64::from(p.full_avg10))),
                ("full_avg60", Value::Integer(i64::from(p.full_avg60))),
            ]),
        ));
    }

    // **Each kernel line present or absent on its own.** A kernel built without
    // CMA writes no `CmaTotal`, and reporting zero would say it reserved none.
    let k = read.kernel;
    for (name, value) in [
        ("slab_bytes", k.slab_bytes),
        ("slab_reclaimable_bytes", k.slab_reclaimable_bytes),
        ("slab_unreclaimable_bytes", k.slab_unreclaimable_bytes),
        ("shmem_bytes", k.shmem_bytes),
        ("page_tables_bytes", k.page_tables_bytes),
        ("kernel_stack_bytes", k.kernel_stack_bytes),
        ("vmalloc_used_bytes", k.vmalloc_used_bytes),
        ("cma_total_bytes", k.cma_total_bytes),
        ("cma_free_bytes", k.cma_free_bytes),
    ] {
        if let Some(n) = value {
            fields.push((name, bytes(n)));
        }
    }

    // **Absent when unread; a refusal when unparseable; never a partial list.**
    match read.slab_caches {
        None => {}
        Some(SlabCaches::Refused(why)) => fields.push(("slab_caches_refused", Value::str(why))),
        Some(SlabCaches::Read(caches)) => {
            fields.push(("slab_cache_count", bytes(u64::try_from(caches.len()).unwrap_or(u64::MAX))));
            fields.push(("slab_caches", Value::List(largest(caches).iter().map(cache).collect())));
        }
    }

    debug_assert!(
        super::get::in_declared_order(&fields, OUTPUTS.iter().map(|f| f.name)),
        "a `get memory` field is undeclared or out of order"
    );
    Ok(Value::object(fields))
}

/// The [`SLAB_CACHES_REPORTED`] caches holding the most bytes, largest first.
/// Ties go by name, so two readings of an unchanged machine are the same bytes.
fn largest(mut caches: Vec<SlabCache>) -> Vec<SlabCache> {
    caches.sort_by(|a, b| b.bytes().cmp(&a.bytes()).then_with(|| a.name.cmp(&b.name)));
    caches.truncate(SLAB_CACHES_REPORTED);
    caches
}

/// One cache as the answer carries it.
fn cache(c: &SlabCache) -> Value {
    let n = |v: u64| Value::Integer(i64::try_from(v).unwrap_or(i64::MAX));
    let mut fields = vec![
        ("name", Value::str(c.name.clone())),
        ("active_objects", n(c.active_objects)),
        ("objects", n(c.objects)),
        ("object_size", n(c.object_size)),
        ("bytes", n(c.bytes())),
    ];
    if let Some(merged) = &c.merged {
        fields.push(("merged", Value::strings(merged.iter().cloned())));
    }
    Value::object(fields)
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;
