//! The probe suite.
//!
//! Probe names describe what is actually measured, not the textbook concept
//! they approximate. `thread_pingpong_rtt` is not "context switch latency" — it
//! is a round trip over a rendezvous channel, which includes two switches plus
//! the channel itself. Naming it honestly is cheaper than defending it later.

pub mod branch;
pub mod memory;
pub mod mutex;
pub mod pagefault;
pub mod syscall;
pub mod thread;

use crate::json::J;
use crate::stats::{summarize, Summary};
use std::time::Instant;

/// A probe whose samples spread wider than this is reported but marked
/// unstable, so downstream consumers can drop it without dropping the run.
pub const MAX_PROBE_SPREAD_PCT: f64 = 20.0;

pub struct Probe {
    pub name: &'static str,
    pub unit: &'static str,
    /// One sentence on what the number literally is, carried in the result file
    /// so a reader never has to guess at the methodology.
    pub what: &'static str,
    pub summary: Option<Summary>,
    pub points: Vec<Point>,
    pub note: Option<String>,
}

pub struct Point {
    pub x_label: &'static str,
    pub x: u64,
    pub summary: Summary,
}

impl Probe {
    pub fn skipped(
        name: &'static str,
        unit: &'static str,
        what: &'static str,
        why: String,
    ) -> Probe {
        Probe {
            name,
            unit,
            what,
            summary: None,
            points: Vec::new(),
            note: Some(why),
        }
    }

    pub fn stable(&self) -> bool {
        match (&self.summary, self.points.is_empty()) {
            (Some(s), _) => s.spread_pct <= MAX_PROBE_SPREAD_PCT,
            (None, false) => self.unstable_points().is_empty(),
            (None, true) => false,
        }
    }

    /// The x values whose samples spread past the ceiling.
    ///
    /// Reported point by point rather than as a single verdict on the probe. On
    /// the memory curve the working sets that sit right at a cache's capacity
    /// are genuinely bimodal — part of the set stays resident, part is evicted
    /// between samples — so they spread wide every time, on every machine. That
    /// is a real property of the hardware and the most interesting region of
    /// the curve. Condemning fourteen good points because of it would be wrong;
    /// so would quietly passing the wide one off as solid.
    pub fn unstable_points(&self) -> Vec<u64> {
        self.points
            .iter()
            .filter(|p| p.summary.spread_pct > MAX_PROBE_SPREAD_PCT)
            .map(|p| p.x)
            .collect()
    }

    pub fn to_json(&self) -> J {
        let mut fields = vec![
            ("unit", J::s(self.unit)),
            ("what", J::s(self.what)),
            (
                "measured",
                J::Bool(self.summary.is_some() || !self.points.is_empty()),
            ),
        ];
        if self.summary.is_some() || !self.points.is_empty() {
            fields.push(("stable", J::Bool(self.stable())));
        }
        if let Some(s) = &self.summary {
            fields.push(("median", J::f(s.median)));
            fields.push(("min", J::f(s.min)));
            fields.push(("p10", J::f(s.p10)));
            fields.push(("p90", J::f(s.p90)));
            fields.push(("spread_pct", J::f(s.spread_pct)));
            fields.push(("samples", J::u(s.samples as u64)));
        }
        if !self.points.is_empty() {
            fields.push((
                "unstable_x",
                J::Arr(self.unstable_points().into_iter().map(J::u).collect()),
            ));
            fields.push((
                "curve",
                J::Arr(
                    self.points
                        .iter()
                        .map(|p| {
                            J::Obj(vec![
                                ("x_label", J::s(p.x_label)),
                                ("x", J::u(p.x)),
                                ("median", J::f(p.summary.median)),
                                ("min", J::f(p.summary.min)),
                                ("spread_pct", J::f(p.summary.spread_pct)),
                                (
                                    "stable",
                                    J::Bool(p.summary.spread_pct <= MAX_PROBE_SPREAD_PCT),
                                ),
                                ("samples", J::u(p.summary.samples as u64)),
                            ])
                        })
                        .collect(),
                ),
            ));
        }
        if let Some(note) = &self.note {
            fields.push(("note", J::s(note)));
        }
        J::Obj(fields)
    }
}

/// Run `f` `reps` times and summarize. `f` returns nanoseconds for the whole
/// batch; `ops` is how many operations that batch contained.
pub fn bench(reps: usize, ops: u64, mut f: impl FnMut() -> f64) -> Summary {
    // One unrecorded pass so the first sample does not pay for cold caches,
    // lazy page mapping, and CPU ramp-up on behalf of the whole series.
    f();
    let mut samples = Vec::with_capacity(reps);
    for _ in 0..reps {
        samples.push(f() / ops as f64);
    }
    summarize(&samples)
}

pub fn time_ns(f: impl FnOnce()) -> f64 {
    let t = Instant::now();
    f();
    t.elapsed().as_nanos() as f64
}
