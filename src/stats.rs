//! Summary statistics for probe samples.
//!
//! Every probe reports a median rather than a mean, and a spread rather than a
//! standard deviation. A single descheduled sample can move a mean by an order
//! of magnitude; the whole point of the run guard is that we would rather throw
//! a noisy measurement away than publish it.

#[derive(Clone, Debug)]
pub struct Summary {
    pub samples: usize,
    pub median: f64,
    pub p10: f64,
    pub p90: f64,
    pub min: f64,
    /// Interquartile range as a percentage of the median.
    ///
    /// Deliberately not (p90 - p10): with a couple of dozen samples the 10th
    /// and 90th percentiles sit close enough to the extremes that a single
    /// descheduled run inflates the figure past 100%, which then reads as a
    /// broken probe rather than one preempted sample. The median is robust, so
    /// the dispersion measure paired with it should be too.
    pub spread_pct: f64,
}

pub fn summarize(values: &[f64]) -> Summary {
    assert!(!values.is_empty(), "summarize() called with no samples");
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).expect("probe produced NaN"));

    let median = percentile(&v, 0.50);
    let p10 = percentile(&v, 0.10);
    let p90 = percentile(&v, 0.90);
    let (p25, p75) = (percentile(&v, 0.25), percentile(&v, 0.75));
    let spread_pct = if median > 0.0 {
        (p75 - p25) / median * 100.0
    } else {
        f64::INFINITY
    };

    Summary {
        samples: v.len(),
        median,
        p10,
        p90,
        min: v[0],
        spread_pct,
    }
}

/// Nearest-rank percentile over an already-sorted slice.
fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.len() == 1 {
        return sorted[0];
    }
    let rank = q * (sorted.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    if lo == hi {
        sorted[lo]
    } else {
        let frac = rank - lo as f64;
        sorted[lo] + (sorted[hi] - sorted[lo]) * frac
    }
}

/// xorshift64*. We need a permutation that defeats the prefetcher, not
/// cryptography, and pulling in `rand` would cost us the one-dependency rule.
/// The seed is recorded in the result file so a run can be repeated exactly.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(if seed == 0 { 0x9E3779B97F4A7C15 } else { seed })
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}
