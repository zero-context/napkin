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

/// One outcome of a pre-run check: a reason to refuse outright, or a caveat to
/// record alongside a result that still stands.
#[derive(Debug, PartialEq, Eq)]
pub enum Finding {
    Invalid(String),
    Warn(String),
}

/// Decide what the machine's clock situation means for trusting a result.
///
/// Pure, and unit-tested, because the combinations that matter are ones the
/// author's own machine cannot produce: a bare-metal box with no cpufreq driver
/// at all, and a virtualized runner that exposes no governor. The second of
/// those shipped in v0.1.0 as a mere warning, which meant a shared CI runner
/// could write a committable result file.
pub fn clock_findings(governor: Option<&str>, virtualized: Option<bool>) -> Vec<Finding> {
    let mut out = Vec::new();

    // Note the asymmetry: failing to confirm the clock is pinned is not the
    // same as confirming it is not. The second is a refusal on its own; the
    // first only becomes one in company.
    let pinning_unverified = match governor {
        Some("performance") => false,
        Some(other) => {
            out.push(Finding::Invalid(format!(
                "cpu governor is '{}', not 'performance'; clock ramping makes these numbers \
                 unreproducible. set it with: sudo cpupower frequency-set -g performance",
                other
            )));
            false
        }
        None => true,
    };

    match (virtualized, pinning_unverified) {
        // The case that slipped through v0.1.0. On a shared runner the host
        // owns the clock, the guest cannot pin it, and no governor is exposed
        // to say otherwise. Nothing about the timing environment is verifiable,
        // and the cpu named in the file is not really the cpu that was
        // measured, so refuse rather than publish a number attributed to it.
        (Some(true), true) => out.push(Finding::Invalid(
            "running under a hypervisor with no cpufreq governor exposed: there is no way to \
             confirm the clock was pinned, and the host's other tenants are in these numbers. \
             a result from here cannot honestly be attributed to the cpu it names"
                .into(),
        )),
        // A guest that can at least pin its own clock is still worth having.
        // Cloud instances are the hardware most software actually runs on, and
        // the calibration drift check will catch a host that steals time.
        (Some(true), false) => out.push(Finding::Warn(
            "running under a hypervisor; timings include virtualization overhead and the host's \
             other tenants"
                .into(),
        )),
        // No cpufreq driver on bare metal is normal on machines that do no
        // frequency scaling at all, and those are exactly the unusual machines
        // the dataset most wants. Record the caveat and carry on.
        (_, true) => out.push(Finding::Warn(
            "no cpufreq governor exposed; could not confirm the clock was pinned".into(),
        )),
        (_, false) => {}
    }

    out
}

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

        for finding in clock_findings(m.governor.as_deref(), m.virtualized) {
            match finding {
                Finding::Invalid(msg) => g.invalidations.push(msg),
                Finding::Warn(msg) => g.warnings.push(msg),
            }
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

#[cfg(test)]
mod tests {
    use super::{clock_findings, Finding};

    fn invalids(g: Option<&str>, v: Option<bool>) -> usize {
        clock_findings(g, v)
            .iter()
            .filter(|f| matches!(f, Finding::Invalid(_)))
            .count()
    }

    fn warns(g: Option<&str>, v: Option<bool>) -> usize {
        clock_findings(g, v)
            .iter()
            .filter(|f| matches!(f, Finding::Warn(_)))
            .count()
    }

    #[test]
    fn pinned_bare_metal_is_the_clean_case() {
        assert_eq!(clock_findings(Some("performance"), Some(false)), vec![]);
    }

    #[test]
    fn a_ramping_governor_is_refused_anywhere() {
        for virtualized in [Some(true), Some(false), None] {
            assert_eq!(
                invalids(Some("powersave"), virtualized),
                1,
                "powersave should be refused with virtualized={:?}",
                virtualized
            );
            assert_eq!(invalids(Some("schedutil"), virtualized), 1);
        }
    }

    #[test]
    fn bare_metal_without_cpufreq_is_allowed_with_a_caveat() {
        // Machines that do no frequency scaling at all are exactly the unusual
        // hardware the dataset wants; refusing them would be worse than the bug
        // this rule was written to fix.
        assert_eq!(invalids(None, Some(false)), 0);
        assert_eq!(warns(None, Some(false)), 1);
    }

    #[test]
    fn a_guest_that_pins_its_own_clock_is_allowed_with_a_caveat() {
        assert_eq!(invalids(Some("performance"), Some(true)), 0);
        assert_eq!(warns(Some("performance"), Some(true)), 1);
    }

    #[test]
    fn a_guest_with_no_governor_is_refused() {
        // The v0.1.0 regression: this combination is what a shared CI runner
        // looks like, and it produced a committable result file.
        assert_eq!(
            invalids(None, Some(true)),
            1,
            "a hypervisor guest exposing no governor must not produce a result"
        );
    }

    #[test]
    fn unknown_virtualization_does_not_escalate_to_a_refusal() {
        // Non-Linux platforms report neither field. They should still be able
        // to contribute a flagged result rather than being locked out.
        assert_eq!(invalids(None, None), 0);
        assert_eq!(warns(None, None), 1);
    }
}
