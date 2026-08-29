//! Facts about the machine under test.
//!
//! Deliberately absent: hostname, username, MAC address, serial numbers,
//! network interfaces. A result file is meant to be pasted into a public pull
//! request by someone who has not read it, so it must not be able to carry
//! anything personal in the first place.

use crate::json::J;
use std::fs;

pub struct Machine {
    pub arch: String,
    pub os: String,
    pub kernel: String,
    pub cpu_model: String,
    pub logical_cpus: usize,
    pub physical_cores: Option<usize>,
    pub ram_bytes: Option<u64>,
    pub governor: Option<String>,
    pub virtualized: Option<bool>,
    pub caches: Vec<Cache>,
}

pub struct Cache {
    pub level: u32,
    pub kind: String,
    pub bytes: u64,
}

/// Pin the calling thread to one CPU for the duration of a probe, returning the
/// previous affinity mask so it can be restored.
///
/// Migration between cores mid-measurement is the largest single source of
/// outliers on an otherwise idle machine: the new core has cold caches and, on
/// a multi-socket box, different memory locality. Pinning is standard practice
/// for microbenchmarks and the chosen cpu is recorded in the result.
#[cfg(target_os = "linux")]
pub struct Affinity(libc::cpu_set_t);

#[cfg(target_os = "linux")]
pub fn pin_to(cpu: usize) -> Option<Affinity> {
    unsafe {
        let mut previous: libc::cpu_set_t = std::mem::zeroed();
        if libc::sched_getaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &mut previous) != 0 {
            return None;
        }
        let mut want: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_ZERO(&mut want);
        libc::CPU_SET(cpu, &mut want);
        if libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &want) != 0 {
            return None;
        }
        Some(Affinity(previous))
    }
}

#[cfg(target_os = "linux")]
impl Drop for Affinity {
    fn drop(&mut self) {
        unsafe {
            libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &self.0);
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub struct Affinity;

#[cfg(not(target_os = "linux"))]
pub fn pin_to(_cpu: usize) -> Option<Affinity> {
    None
}

pub fn collect() -> Machine {
    Machine {
        arch: std::env::consts::ARCH.to_string(),
        os: std::env::consts::OS.to_string(),
        kernel: read_trim("/proc/sys/kernel/osrelease").unwrap_or_else(|| "unknown".into()),
        cpu_model: cpu_model(),
        logical_cpus: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1),
        physical_cores: physical_cores(),
        ram_bytes: ram_bytes(),
        governor: read_trim("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"),
        virtualized: virtualized(),
        caches: caches(),
    }
}

fn read_trim(path: &str) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn cpu_model() -> String {
    // x86 exposes "model name"; arm64 has no equivalent, so fall back through
    // the fields those platforms do publish before giving up.
    if let Ok(info) = fs::read_to_string("/proc/cpuinfo") {
        for key in ["model name", "Model", "cpu model", "Hardware"] {
            for line in info.lines() {
                if let Some((k, v)) = line.split_once(':') {
                    if k.trim() == key && !v.trim().is_empty() {
                        return v.trim().to_string();
                    }
                }
            }
        }
    }
    if let Some(compat) = read_trim("/proc/device-tree/model") {
        return compat.trim_end_matches('\0').to_string();
    }
    "unknown".to_string()
}

fn physical_cores() -> Option<usize> {
    let info = fs::read_to_string("/proc/cpuinfo").ok()?;
    let mut seen = std::collections::BTreeSet::new();
    let (mut pkg, mut core) = (None, None);
    for line in info.lines() {
        if let Some((k, v)) = line.split_once(':') {
            match k.trim() {
                "physical id" => pkg = v.trim().parse::<u32>().ok(),
                "core id" => core = v.trim().parse::<u32>().ok(),
                _ => {}
            }
        }
        if line.trim().is_empty() {
            if let (Some(p), Some(c)) = (pkg, core) {
                seen.insert((p, c));
            }
            pkg = None;
            core = None;
        }
    }
    if let (Some(p), Some(c)) = (pkg, core) {
        seen.insert((p, c));
    }
    if seen.is_empty() {
        None
    } else {
        Some(seen.len())
    }
}

fn ram_bytes() -> Option<u64> {
    let info = fs::read_to_string("/proc/meminfo").ok()?;
    for line in info.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            let kb: u64 = rest.trim().trim_end_matches(" kB").trim().parse().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

fn virtualized() -> Option<bool> {
    if let Ok(info) = fs::read_to_string("/proc/cpuinfo") {
        for line in info.lines() {
            if line.starts_with("flags") && line.contains(" hypervisor") {
                return Some(true);
            }
        }
        if std::path::Path::new("/sys/hypervisor/type").exists() {
            return Some(true);
        }
        // DMI vendor strings are the fallback for hypervisors that hide the
        // CPUID bit.
        if let Some(v) = read_trim("/sys/class/dmi/id/sys_vendor") {
            let v = v.to_ascii_lowercase();
            for needle in [
                "qemu",
                "vmware",
                "innotek",
                "xen",
                "microsoft corporation",
                "parallels",
            ] {
                if v.contains(needle) {
                    return Some(true);
                }
            }
        }
        return Some(false);
    }
    None
}

fn caches() -> Vec<Cache> {
    let mut out = Vec::new();
    for i in 0..10 {
        let base = format!("/sys/devices/system/cpu/cpu0/cache/index{}", i);
        let Some(level) = read_trim(&format!("{}/level", base)).and_then(|s| s.parse().ok()) else {
            continue;
        };
        let Some(kind) = read_trim(&format!("{}/type", base)) else {
            continue;
        };
        let Some(size) = read_trim(&format!("{}/size", base)) else {
            continue;
        };
        // sysfs writes sizes as "32K" / "3072K" / "8M".
        let bytes = match size.chars().last() {
            Some('K') => size[..size.len() - 1].parse::<u64>().ok().map(|v| v * 1024),
            Some('M') => size[..size.len() - 1]
                .parse::<u64>()
                .ok()
                .map(|v| v * 1024 * 1024),
            _ => size.parse::<u64>().ok(),
        };
        if let Some(bytes) = bytes {
            out.push(Cache { level, kind, bytes });
        }
    }
    out
}

impl Machine {
    pub fn to_json(&self) -> J {
        J::Obj(vec![
            ("arch", J::s(&self.arch)),
            ("os", J::s(&self.os)),
            ("kernel", J::s(&self.kernel)),
            ("cpu_model", J::s(&self.cpu_model)),
            ("logical_cpus", J::u(self.logical_cpus as u64)),
            (
                "physical_cores",
                self.physical_cores
                    .map(|c| J::u(c as u64))
                    .unwrap_or(J::Null),
            ),
            ("ram_bytes", self.ram_bytes.map(J::u).unwrap_or(J::Null)),
            (
                "governor",
                self.governor.as_deref().map(J::s).unwrap_or(J::Null),
            ),
            (
                "virtualized",
                self.virtualized.map(J::Bool).unwrap_or(J::Null),
            ),
            (
                "caches",
                J::Arr(
                    self.caches
                        .iter()
                        .map(|c| {
                            J::Obj(vec![
                                ("level", J::u(c.level as u64)),
                                ("type", J::s(&c.kind)),
                                ("bytes", J::u(c.bytes)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }

    /// Filename stem for the result file: `<arch>/<slugged-cpu>-<n>cpu`.
    pub fn slug(&self) -> String {
        let mut s = String::new();
        for c in self.cpu_model.to_ascii_lowercase().chars() {
            if c.is_ascii_alphanumeric() {
                s.push(c);
            } else if !s.ends_with('-') {
                s.push('-');
            }
        }
        let s = s.trim_matches('-').to_string();
        format!("{}-{}cpu", s, self.logical_cpus)
    }
}
