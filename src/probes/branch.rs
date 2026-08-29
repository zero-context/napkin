//! Branch misprediction penalty.
//!
//! Measured as a difference, not an absolute: the same array of values is
//! walked twice, once sorted and once shuffled. Identical data, identical
//! number of taken branches, identical memory traffic — the only thing that
//! changes is whether the predictor can see the pattern. The delta is the
//! penalty.
//!
//! The branch body calls a `#[inline(never)]` function so that LLVM cannot
//! quietly rewrite the whole loop into a branchless conditional move, which
//! would measure nothing at all.

use super::{bench, time_ns, Probe};
use crate::stats::Rng;
use std::hint::black_box;

const N: usize = 1 << 16;
const REPS: usize = 15;

#[inline(never)]
fn taken(acc: u64, v: u8) -> u64 {
    acc.wrapping_add(v as u64)
}

fn walk(data: &[u8]) -> u64 {
    let mut acc = 0u64;
    for &v in data {
        if v >= 128 {
            acc = taken(acc, v);
        }
    }
    acc
}

pub fn run(seed: u64) -> Probe {
    let mut rng = Rng::new(seed ^ 0xB1A5);
    let mut data: Vec<u8> = (0..N).map(|_| (rng.next_u64() & 0xFF) as u8).collect();

    let shuffled = data.clone();
    data.sort_unstable();
    let sorted = data;

    // The array is 64KiB and both walks are sequential, so it is L1/L2-resident
    // in both cases; the difference is not a cache effect.
    let s_sorted = bench(REPS, N as u64, || {
        time_ns(|| {
            black_box(walk(black_box(&sorted)));
        })
    });
    let s_shuffled = bench(REPS, N as u64, || {
        time_ns(|| {
            black_box(walk(black_box(&shuffled)));
        })
    });

    let delta = s_shuffled.median - s_sorted.median;
    // Roughly half the elements clear the threshold, and only those are
    // mispredictable, so the per-element delta understates the per-miss cost by
    // about 2x. Report per element and say so rather than applying a fudge.
    let note = format!(
        "sorted walk {:.2} ns/element (spread {:.1}%), shuffled walk {:.2} ns/element (spread \
         {:.1}%). roughly half the elements are branch-taken, so the per-misprediction cost is \
         near twice this delta.",
        s_sorted.median, s_sorted.spread_pct, s_shuffled.median, s_shuffled.spread_pct
    );

    let mut summary = s_shuffled.clone();
    summary.median = delta;
    summary.min = s_shuffled.min - s_sorted.min;
    summary.p10 = s_shuffled.p10 - s_sorted.p10;
    summary.p90 = s_shuffled.p90 - s_sorted.p90;
    // Spread of a difference is not the spread of either term; carry the worse
    // of the two so an unstable input cannot look like a clean result.
    summary.spread_pct = s_sorted.spread_pct.max(s_shuffled.spread_pct);

    Probe {
        name: "branch_mispredict_penalty",
        unit: "ns",
        what: WHAT,
        summary: Some(summary),
        points: Vec::new(),
        note: Some(note),
    }
}

const WHAT: &str =
    "nanoseconds per element by which a shuffled walk is slower than a sorted walk over the same \
     64KiB array, with the same branch taken the same number of times. this is the cost of an \
     unpredictable branch, averaged over all elements.";
