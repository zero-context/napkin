//! First-touch minor page fault.
//!
//! A fresh anonymous mapping costs nothing until it is written to; the kernel
//! then finds a page, zeroes it, and installs the mapping. That is the cost
//! behind every large allocation that "should have been free", and it is the
//! reason a freshly started process is slower than a warm one.

use super::{bench, time_ns, Probe};

const PAGES: usize = 8192;
const REPS: usize = 9;

pub fn run() -> Probe {
    if cfg!(not(target_os = "linux")) {
        return Probe::skipped(
            "page_fault_first_touch",
            "ns",
            WHAT,
            "mmap-based probe is currently linux-only".into(),
        );
    }

    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page <= 0 {
        return Probe::skipped(
            "page_fault_first_touch",
            "ns",
            WHAT,
            "could not read page size".into(),
        );
    }
    let page = page as usize;
    let len = PAGES * page;

    let summary = bench(REPS, PAGES as u64, || {
        // A fresh mapping every repetition: once a page is touched it stays
        // mapped, so reusing the region would measure a plain store instead.
        let addr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE,
                -1,
                0,
            )
        };
        if addr == libc::MAP_FAILED {
            // Surfaced rather than swallowed: a failed mapping must not be
            // averaged in as a very fast page fault.
            panic!(
                "mmap of {} bytes failed during page-fault probe: {}",
                len,
                std::io::Error::last_os_error()
            );
        }
        let base = addr as *mut u8;
        let ns = time_ns(|| {
            for i in 0..PAGES {
                unsafe { std::ptr::write_volatile(base.add(i * page), 1u8) };
            }
        });
        let rc = unsafe { libc::munmap(addr, len) };
        assert_eq!(rc, 0, "munmap failed: {}", std::io::Error::last_os_error());
        ns
    });

    Probe {
        name: "page_fault_first_touch",
        unit: "ns",
        what: WHAT,
        summary: Some(summary),
        points: Vec::new(),
        note: None,
    }
}

const WHAT: &str =
    "nanoseconds for one minor page fault: the first write to a page of a fresh anonymous \
     MAP_PRIVATE mapping, including the kernel zeroing the page.";
