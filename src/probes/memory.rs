//! Memory access latency as a function of working-set size.
//!
//! This is the probe the project exists for: one curve that shows where this
//! machine's cache levels actually sit and what a miss to DRAM actually costs,
//! rather than the 2012 figures everyone still quotes.
//!
//! Method: a pointer chase around a single random cycle of 64-byte-spaced
//! slots. Every access depends on the previous one, so the loads cannot be
//! overlapped, and a random cycle gives the hardware prefetcher nothing to
//! latch onto. The cycle is a single cycle by construction, so the chase can
//! never fall out of the working set.

use super::{bench, time_ns, Point, Probe};
use crate::stats::Rng;
use std::hint::black_box;

const LINE: usize = 64;
const REPS: usize = 13;

pub fn run(ram_bytes: Option<u64>, seed: u64) -> Probe {
    // Never size the largest working set above an eighth of RAM: the point is
    // to measure DRAM latency, not to start swapping.
    let ceiling = ram_bytes
        .map(|r| (r / 8) as usize)
        .unwrap_or(64 << 20)
        .min(128 << 20);

    let sizes: Vec<usize> = [
        4 << 10,
        16 << 10,
        32 << 10,
        64 << 10,
        128 << 10,
        256 << 10,
        512 << 10,
        1 << 20,
        2 << 20,
        4 << 20,
        8 << 20,
        16 << 20,
        32 << 20,
        64 << 20,
        128 << 20,
    ]
    .into_iter()
    .filter(|&s| s <= ceiling && s >= 4 * LINE)
    .collect();

    if sizes.is_empty() {
        return Probe::skipped(
            "memory_latency_curve",
            "ns",
            WHAT,
            "not enough usable RAM to size a working set".into(),
        );
    }

    let mut rng = Rng::new(seed);
    let mut points = Vec::new();

    for size in sizes {
        let slots = size / LINE;
        let words = size / std::mem::size_of::<usize>();
        let stride = LINE / std::mem::size_of::<usize>();

        // Fisher-Yates over the slot order, then link each slot to the next one
        // in that order, closing the loop. One cycle, every slot visited once.
        let mut order: Vec<usize> = (0..slots).collect();
        for i in (1..slots).rev() {
            let j = rng.below(i + 1);
            order.swap(i, j);
        }
        let mut buf = vec![0usize; words];
        for k in 0..slots {
            buf[order[k] * stride] = order[(k + 1) % slots] * stride;
        }

        // Enough steps to swamp timer overhead, but bounded so a 128MiB run
        // does not take a minute on a slow machine.
        let steps: u64 = (slots as u64 * 8).clamp(200_000, 4_000_000);
        let start = order[0] * stride;

        let summary = bench(REPS, steps, || {
            time_ns(|| {
                // No black_box inside the loop: the chase is a genuine
                // dependent chain through a heap buffer LLVM cannot see into,
                // so it cannot be reordered or elided, and a barrier on every
                // iteration would add several cycles to an L1 hit we are trying
                // to measure at four.
                let mut p = start;
                for _ in 0..steps {
                    p = buf[p];
                }
                black_box(p);
            })
        });

        points.push(Point {
            x_label: "working_set_bytes",
            x: size as u64,
            summary,
        });
    }

    Probe {
        name: "memory_latency_curve",
        unit: "ns",
        what: WHAT,
        summary: None,
        points,
        note: None,
    }
}

const WHAT: &str =
    "nanoseconds for one dependent load, chasing a random 64-byte-strided pointer cycle of the \
     given working-set size. cache-level plateaus and the DRAM step are read off this curve.";
