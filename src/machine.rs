//! Facts about the machine under test.
//!
//! Deliberately absent: hostname, username, MAC address, serial numbers,
//! network interfaces. A result file is meant to be pasted into a public pull
//! request by someone who has not read it, so it must not be able to carry
//! anything personal in the first place.
//!
//! Every platform-specific lookup lives in one of the `facts` modules below,
//! which all expose the same set of functions. Adding a platform means adding a
//! module, not threading `cfg` through the rest of the program.

use crate::json::J;

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

pub fn collect() -> Machine {
    Machine {
        arch: std::env::consts::ARCH.to_string(),
        os: std::env::consts::OS.to_string(),
        kernel: facts::kernel(),
        cpu_model: facts::cpu_model(),
        // Portable, and correct on every platform we support.
        logical_cpus: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1),
        physical_cores: facts::physical_cores(),
        ram_bytes: facts::ram_bytes(),
        governor: facts::governor(),
        virtualized: facts::virtualized(),
        caches: facts::caches(),
    }
}

/// Assemble a cache list from sizes reported one level at a time.
///
/// Shared by the platforms that publish sizes rather than a topology tree, and
/// pure so it can be tested without one of those machines to hand.
pub fn caches_from_sizes(l1d: Option<u64>, l2: Option<u64>, l3: Option<u64>) -> Vec<Cache> {
    let mut out = Vec::new();
    // A reported size of zero means "this level does not exist here" — Apple
    // silicon reports no L3 at all — and must not become a 0-byte cache entry
    // that the site would then try to read a latency off.
    for (level, kind, bytes) in [(1, "Data", l1d), (2, "Unified", l2), (3, "Unified", l3)] {
        if let Some(bytes) = bytes {
            if bytes > 0 {
                out.push(Cache {
                    level,
                    kind: kind.to_string(),
                    bytes,
                });
            }
        }
    }
    out
}

/// Decode a NUL-terminated, NUL-padded C string from a raw byte buffer.
///
/// Only the macOS module calls this, but it is compiled and unit-tested on
/// every platform, which is the point: the parsing stays verified on machines
/// where there is no Mac to run it against.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn trim_nul(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).trim().to_string()
}

// ---------------------------------------------------------------- linux ----

#[cfg(target_os = "linux")]
mod facts {
    use super::{caches_from_sizes, Cache};
    use std::fs;

    pub fn read_trim(path: &str) -> Option<String> {
        fs::read_to_string(path).ok().map(|s| s.trim().to_string())
    }

    pub fn kernel() -> String {
        read_trim("/proc/sys/kernel/osrelease").unwrap_or_else(|| "unknown".into())
    }

    pub fn governor() -> Option<String> {
        read_trim("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
    }

    pub fn cpu_model() -> String {
        // x86 exposes "model name"; arm64 has no equivalent, so fall back
        // through the fields those platforms do publish before giving up.
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

    pub fn physical_cores() -> Option<usize> {
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

    pub fn ram_bytes() -> Option<u64> {
        let info = fs::read_to_string("/proc/meminfo").ok()?;
        for line in info.lines() {
            if let Some(rest) = line.strip_prefix("MemTotal:") {
                let kb: u64 = rest.trim().trim_end_matches(" kB").trim().parse().ok()?;
                return Some(kb * 1024);
            }
        }
        None
    }

    pub fn virtualized() -> Option<bool> {
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

    pub fn caches() -> Vec<Cache> {
        let mut out = Vec::new();
        for i in 0..10 {
            let base = format!("/sys/devices/system/cpu/cpu0/cache/index{}", i);
            let Some(level) = read_trim(&format!("{}/level", base)).and_then(|s| s.parse().ok())
            else {
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
        // Fall back to the flat keys some kernels expose when the topology tree
        // is missing, rather than reporting a machine with no caches at all.
        if out.is_empty() {
            return caches_from_sizes(None, None, None);
        }
        out
    }
}

// ---------------------------------------------------------------- macos ----

#[cfg(target_os = "macos")]
mod facts {
    use super::{caches_from_sizes, trim_nul, Cache};
    use std::ffi::CString;

    /// Read a sysctl string by name.
    fn sysctl_string(name: &str) -> Option<String> {
        let cname = CString::new(name).ok()?;
        let mut len: libc::size_t = 0;
        unsafe {
            // First call sizes the buffer; a failure here means the key does
            // not exist on this machine, which is normal and not an error.
            if libc::sysctlbyname(
                cname.as_ptr(),
                std::ptr::null_mut(),
                &mut len,
                std::ptr::null_mut(),
                0,
            ) != 0
                || len == 0
            {
                return None;
            }
            let mut buf = vec![0u8; len];
            if libc::sysctlbyname(
                cname.as_ptr(),
                buf.as_mut_ptr() as *mut libc::c_void,
                &mut len,
                std::ptr::null_mut(),
                0,
            ) != 0
            {
                return None;
            }
            Some(trim_nul(&buf))
        }
    }

    /// Read a sysctl integer.
    ///
    /// Width varies by key — `hw.memsize` is 64-bit, `hw.physicalcpu` is 32 —
    /// so take the width from what the kernel reports rather than assuming.
    fn sysctl_int(name: &str) -> Option<u64> {
        let cname = CString::new(name).ok()?;
        let mut len: libc::size_t = 0;
        unsafe {
            if libc::sysctlbyname(
                cname.as_ptr(),
                std::ptr::null_mut(),
                &mut len,
                std::ptr::null_mut(),
                0,
            ) != 0
            {
                return None;
            }
            match len {
                4 => {
                    let mut v: u32 = 0;
                    let mut l = len;
                    if libc::sysctlbyname(
                        cname.as_ptr(),
                        &mut v as *mut u32 as *mut libc::c_void,
                        &mut l,
                        std::ptr::null_mut(),
                        0,
                    ) != 0
                    {
                        return None;
                    }
                    Some(v as u64)
                }
                8 => {
                    let mut v: u64 = 0;
                    let mut l = len;
                    if libc::sysctlbyname(
                        cname.as_ptr(),
                        &mut v as *mut u64 as *mut libc::c_void,
                        &mut l,
                        std::ptr::null_mut(),
                        0,
                    ) != 0
                    {
                        return None;
                    }
                    Some(v)
                }
                _ => None,
            }
        }
    }

    pub fn kernel() -> String {
        sysctl_string("kern.osrelease").unwrap_or_else(|| "unknown".into())
    }

    /// macOS has no equivalent of a cpufreq governor: the clock is the
    /// kernel's business and cannot be pinned from userspace. Always `None`,
    /// which the run guard treats as "could not confirm" rather than "wrong".
    pub fn governor() -> Option<String> {
        None
    }

    pub fn cpu_model() -> String {
        // brand_string is present on Apple silicon as well as Intel, where it
        // reads "Apple M2 Pro". hw.model ("Mac14,10") is the backstop: less
        // informative, but the dataset is indexed by this field, so returning
        // "unknown" would make the machine impossible to file.
        sysctl_string("machdep.cpu.brand_string")
            .or_else(|| sysctl_string("hw.model"))
            .unwrap_or_else(|| "unknown".into())
    }

    pub fn physical_cores() -> Option<usize> {
        sysctl_int("hw.physicalcpu").map(|v| v as usize)
    }

    pub fn ram_bytes() -> Option<u64> {
        sysctl_int("hw.memsize")
    }

    pub fn virtualized() -> Option<bool> {
        sysctl_int("kern.hv_vmm_present").map(|v| v == 1)
    }

    pub fn caches() -> Vec<Cache> {
        // Apple silicon splits caches by performance level, and the P-cores
        // (perflevel0) are what a benchmark thread lands on by default. Intel
        // Macs publish the flat keys instead. There is no L3 key on Apple
        // silicon; the system level cache is not exposed, so it is reported as
        // absent rather than guessed at.
        let l1d =
            sysctl_int("hw.perflevel0.l1dcachesize").or_else(|| sysctl_int("hw.l1dcachesize"));
        let l2 = sysctl_int("hw.perflevel0.l2cachesize").or_else(|| sysctl_int("hw.l2cachesize"));
        let l3 = sysctl_int("hw.perflevel0.l3cachesize").or_else(|| sysctl_int("hw.l3cachesize"));
        caches_from_sizes(l1d, l2, l3)
    }
}

// ------------------------------------------------------- everywhere else ----

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod facts {
    use super::Cache;

    pub fn kernel() -> String {
        "unknown".to_string()
    }
    pub fn governor() -> Option<String> {
        None
    }
    pub fn cpu_model() -> String {
        "unknown".to_string()
    }
    pub fn physical_cores() -> Option<usize> {
        None
    }
    pub fn ram_bytes() -> Option<u64> {
        None
    }
    pub fn virtualized() -> Option<bool> {
        None
    }
    pub fn caches() -> Vec<Cache> {
        Vec::new()
    }
}

// ------------------------------------------------------------- affinity ----

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

/// Placeholder for platforms with no way to bind a thread to a core. macOS
/// exposes only affinity *hints*, which the scheduler is free to ignore, so
/// there is nothing honest to return here — the run records that pinning was
/// unavailable instead.
#[cfg(not(target_os = "linux"))]
pub struct Affinity;

#[cfg(not(target_os = "linux"))]
pub fn pin_to(_cpu: usize) -> Option<Affinity> {
    None
}

// ----------------------------------------------------------------- json ----

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

#[cfg(test)]
mod tests {
    use super::{caches_from_sizes, trim_nul};

    #[test]
    fn c_strings_are_decoded_without_their_padding() {
        // sysctl returns a NUL-terminated buffer sized to the whole allocation,
        // so the padding has to come off or the cpu model carries junk into the
        // filename slug.
        assert_eq!(trim_nul(b"Apple M2 Pro\0\0\0\0"), "Apple M2 Pro");
        assert_eq!(trim_nul(b"Apple M2 Pro"), "Apple M2 Pro");
        assert_eq!(trim_nul(b"  spaced  \0"), "spaced");
        assert_eq!(trim_nul(b"\0"), "");
        assert_eq!(trim_nul(b""), "");
    }

    #[test]
    fn apple_silicon_reports_no_l3_rather_than_an_empty_one() {
        // The system level cache is not exposed, and a 0-byte L3 would be read
        // by the site as a cache level with a latency.
        let c = caches_from_sizes(Some(131_072), Some(16_777_216), None);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].level, 1);
        assert_eq!(c[0].bytes, 131_072);
        assert_eq!(c[1].level, 2);
        assert_eq!(c[1].bytes, 16_777_216);
    }

    #[test]
    fn a_zero_size_is_treated_as_absent() {
        assert_eq!(caches_from_sizes(Some(32_768), Some(0), Some(0)).len(), 1);
        assert!(caches_from_sizes(None, None, None).is_empty());
    }

    #[test]
    fn an_intel_mac_reports_all_three_levels() {
        let c = caches_from_sizes(Some(32_768), Some(262_144), Some(6_291_456));
        assert_eq!(c.len(), 3);
        assert_eq!(c[2].level, 3);
        assert_eq!(c[2].kind, "Unified");
    }
}
