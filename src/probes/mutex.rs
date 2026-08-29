//! Uncontended mutex lock plus unlock.
//!
//! Uncontended is the case worth publishing: it is the tax paid on every
//! lock in a program that is not actually fighting over anything, and on Linux
//! it should never reach the kernel at all.

use super::{bench, time_ns, Probe};
use std::hint::black_box;
use std::sync::Mutex;

const OPS: u64 = 500_000;
const REPS: usize = 11;

pub fn run() -> Probe {
    let m = Mutex::new(0u64);
    let summary = bench(REPS, OPS, || {
        time_ns(|| {
            for _ in 0..OPS {
                let mut g = m.lock().expect("napkin's own mutex was poisoned");
                *g = black_box(*g).wrapping_add(1);
            }
        })
    });
    Probe {
        name: "mutex_lock_unlock_uncontended",
        unit: "ns",
        what: WHAT,
        summary: Some(summary),
        points: Vec::new(),
        note: None,
    }
}

const WHAT: &str =
    "nanoseconds for one lock/unlock pair on an uncontended std::sync::Mutex, including the \
     guarded increment. no other thread ever touches the lock, so this should stay in userspace.";
