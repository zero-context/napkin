//! Cost of the cheapest real system call.
//!
//! `getppid` is used rather than `getpid` because glibc cached `getpid` for
//! years and some libcs still serve it from the vDSO — which would measure a
//! function call, not a kernel entry. `getppid` has never been cacheable: the
//! parent can change under you when it exits.

use super::{bench, time_ns, Probe};
use std::hint::black_box;

const OPS: u64 = 200_000;
const REPS: usize = 11;

pub fn run() -> Probe {
    if cfg!(not(unix)) {
        return Probe::skipped("syscall_getppid", "ns", WHAT, "not a unix platform".into());
    }
    let summary = bench(REPS, OPS, || {
        time_ns(|| {
            for _ in 0..OPS {
                black_box(unsafe { libc::getppid() });
            }
        })
    });
    Probe {
        name: "syscall_getppid",
        unit: "ns",
        what: WHAT,
        summary: Some(summary),
        points: Vec::new(),
        note: None,
    }
}

const WHAT: &str =
    "nanoseconds for one getppid(2) round trip into the kernel and back. this is a floor on \
     syscall cost, and it moved sharply with the 2018 speculative-execution mitigations.";
