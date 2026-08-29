//! The run guard: the reason anyone should believe a napkin result.
//!
//! Trust in the dataset is the product. A number measured on a thermally
//! throttled laptop that was also compiling something is worse than no number,
//! because it is indistinguishable from a good one once it is committed. So the
//! guard's job is to refuse: it would rather emit `"valid": false` and a reason
//! than let a plausible-looking measurement into `results/`.

use crate::json::J;
use crate::machine::Machine;
use std::hint::black_box;
use std::time::Instant;

/// Fail the run if the calibration workload slows by more than this between the
/// start and the end of the probe suite. On a laptop that is nearly always
/// thermal throttling; whatever the cause, the probes either side of the drift
/// were not measuring the same machine.
const MAX_CALIBRATION_DRIFT_PCT: f64 = 5.0;

/// Fail the run if 1-minute load average exceeds this fraction of the logical
/// CPU count at start. Something else was running, so the memory curve is
/// somebody else's cache footprint as much as ours.
const MAX_LOAD_FRACTION: f64 = 0.30;

pub struct Guard {
    iters: u64,
    start_ns: f64,
    end_ns: Option<f64>,
    pub invalidations: Vec<String>,
    pub warnings: Vec<String>,
}

impl Guard {
    pub fn start(m: &Machine) -> Guard {
        let mut g = Guard {
            iters: 0,
            start_ns: 0.0,
            end_ns: None,
            invalidations: Vec::new(),
            warnings: Vec::new(),
        };

        match m.governor.as_deref() {
            Some("performance") => {}
            Some(other) => g.invalidations.push(format!(
                "cpu governor is '{}', not 'performance'; clock ramping makes these numbers \
                 unreproducible. set it with: sudo cpupower frequency-set -g performance",
                other
            )),
            None => g
                .warnings
                .push("no cpufreq governor exposed; could not confirm the clock was pinned".into()),
        }

        if m.virtualized == Some(true) {
            g.warnings.push(
                "running under a hypervisor; timings include virtualization overhead and the \
                 host's other tenants"
                    .into(),
            );
        }
        if m.os != "linux" {
            g.warnings.push(format!(
                "os is '{}'; machine facts and some probes are Linux-only and were skipped",
                m.os
            ));
        }

        if let Some(load1) = loadavg_1min() {
            let ceiling = m.logical_cpus as f64 * MAX_LOAD_FRACTION;
            if load1 > ceiling {
                g.invalidations.push(format!(
                    "1-minute load average was {:.2} at start, above the {:.2} ceiling for {} \
                     logical cpus; close other work and rerun",
                    load1, ceiling, m.logical_cpus
                ));
            }
        } else {
            g.warnings
                .push("could not read /proc/loadavg; machine idleness unverified".into());
        }

        // Size the calibration workload to roughly 200ms on this machine, then
        // reuse that exact iteration count at the end so the two timings are
        // directly comparable.
        g.iters = size_calibration();
        g.start_ns = calibrate(g.iters);
        g
    }

    pub fn finish(&mut self) {
        let end_ns = calibrate(self.iters);
        self.end_ns = Some(end_ns);
        let drift = (end_ns - self.start_ns) / self.start_ns * 100.0;
        if drift > MAX_CALIBRATION_DRIFT_PCT {
            self.invalidations.push(format!(
                "the machine got {:.1}% slower during the run (calibration drift, ceiling {:.1}%); \
                 almost certainly thermal or power throttling. let it cool and rerun",
                drift, MAX_CALIBRATION_DRIFT_PCT
            ));
        }
    }

    pub fn valid(&self) -> bool {
        self.invalidations.is_empty()
    }

    pub fn drift_pct(&self) -> Option<f64> {
        self.end_ns
            .map(|e| (e - self.start_ns) / self.start_ns * 100.0)
    }

    pub fn to_json(&self) -> J {
        J::Obj(vec![
            ("calibration_start_ns", J::f(self.start_ns)),
            (
                "calibration_end_ns",
                self.end_ns.map(J::f).unwrap_or(J::Null),
            ),
            (
                "calibration_drift_pct",
                self.drift_pct().map(J::f).unwrap_or(J::Null),
            ),
            ("max_drift_pct", J::f(MAX_CALIBRATION_DRIFT_PCT)),
        ])
    }
}

/// A dependent integer chain: no memory traffic, no branches to predict, so its
/// runtime tracks core clock and nothing else.
#[inline(never)]
fn calibration_work(iters: u64) -> u64 {
    let mut x: u64 = 0x243F6A8885A308D3;
    for _ in 0..iters {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        x ^= x >> 31;
    }
    x
}

fn calibrate(iters: u64) -> f64 {
    let t = Instant::now();
    black_box(calibration_work(black_box(iters)));
    t.elapsed().as_nanos() as f64
}

fn size_calibration() -> u64 {
    let mut iters = 1 << 16;
    loop {
        let ns = calibrate(iters);
        if ns > 200_000_000.0 || iters > 1 << 34 {
            return iters;
        }
        // Scale toward the 200ms target rather than doubling blindly, so a fast
        // machine does not spend a dozen rounds getting there.
        let factor = (200_000_000.0 / ns.max(1.0)).clamp(2.0, 16.0);
        iters = (iters as f64 * factor) as u64;
    }
}

fn loadavg_1min() -> Option<f64> {
    let s = std::fs::read_to_string("/proc/loadavg").ok()?;
    s.split_whitespace().next()?.parse().ok()
}
