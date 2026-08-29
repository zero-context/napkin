//! napkin — measured latency numbers for hardware that actually exists.
//!
//! Run it, and it writes one JSON file describing this machine. Open a pull
//! request with that file. That is the whole contribution protocol.

mod guard;
mod json;
mod machine;
mod probes;
mod stats;

use guard::Guard;
use json::J;
use probes::Probe;
use std::time::{SystemTime, UNIX_EPOCH};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const SCHEMA: u64 = 1;

/// Which C library this binary was linked against.
///
/// Recorded because the project ships static musl builds alongside
/// build-from-source runs that link the system glibc, and the two are not
/// guaranteed to measure identically at the syscall and mmap boundary. If a
/// systematic difference ever appears in the dataset it has to be attributable
/// rather than invisible — the same reason the guard records its conditions
/// instead of silently passing.
const LIBC: &str = if cfg!(target_env = "musl") {
    "musl"
} else if cfg!(target_env = "gnu") {
    "gnu"
} else if cfg!(target_os = "macos") {
    "libsystem"
} else {
    "unknown"
};

struct Args {
    out: Option<String>,
    stdout: bool,
    seed: u64,
}

fn main() {
    let args = match parse_args() {
        Ok(Some(a)) => a,
        Ok(None) => return,
        Err(e) => {
            eprintln!("napkin: {}", e);
            std::process::exit(2);
        }
    };

    let m = machine::collect();
    eprintln!("napkin {} — {}", VERSION, m.cpu_model);
    eprintln!(
        "  {} logical cpus{}, {} kernel {}",
        m.logical_cpus,
        m.physical_cores
            .map(|c| format!(" / {} cores", c))
            .unwrap_or_default(),
        m.arch,
        m.kernel
    );
    eprintln!();

    let mut g = Guard::start(&m);
    // Refuse before spending two minutes on measurements that are already
    // known to be worthless.
    if !g.valid() {
        report_invalid(&g);
        std::process::exit(1);
    }

    // Highest-numbered cpu: lowest-numbered ones carry most of the interrupt
    // load on a typical Linux box.
    let pin_cpu = m.logical_cpus.saturating_sub(1);
    let mut pinned_cpu = None;

    let mut probes = Vec::new();
    for (label, wants_all_cpus, run) in probe_suite(&m, args.seed) {
        // Held for the duration of one probe and dropped after, restoring the
        // original mask. The thread probe opts out: pinning both of its threads
        // to one cpu would turn a hand-off between cores into a forced
        // context switch and measure something else entirely.
        let held = if wants_all_cpus {
            None
        } else {
            machine::pin_to(pin_cpu)
        };
        if held.is_some() {
            pinned_cpu = Some(pin_cpu);
        } else if !wants_all_cpus && pinned_cpu.is_none() {
            pinned_cpu = Some(usize::MAX);
        }
        eprint!("  {:<32}", label);
        let p = run();
        drop(held);
        eprintln!("{}", one_line(&p));
        probes.push(p);
    }

    if pinned_cpu == Some(usize::MAX) {
        g.warnings.push(
            "could not pin probes to a cpu; thread migration mid-probe may widen the spread".into(),
        );
        pinned_cpu = None;
    }
    eprintln!();

    g.finish();

    let doc = build(&m, &g, &probes, args.seed, pinned_cpu);
    let text = doc.to_string();

    if !g.valid() {
        report_invalid(&g);
        eprintln!("not writing a result file. nothing above should be trusted.");
        std::process::exit(1);
    }

    for w in &g.warnings {
        eprintln!("warning: {}", w);
    }
    for p in &probes {
        if p.summary.is_some() && !p.stable() {
            eprintln!(
                "note: {} spread past the {:.0}% ceiling and is flagged unstable in the result",
                p.name,
                probes::MAX_PROBE_SPREAD_PCT
            );
        }
        let wide = p.unstable_points();
        if !wide.is_empty() {
            eprintln!(
                "note: {} is bimodal at {} — expected where a working set sits at a cache's \
                 capacity; those points are flagged, the rest of the curve stands",
                p.name,
                wide.iter()
                    .map(|x| human_bytes(*x))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }

    if args.stdout {
        print!("{}", text);
        return;
    }

    let path = args
        .out
        .unwrap_or_else(|| format!("results/{}/{}.json", m.arch, m.slug()));
    if let Some(dir) = std::path::Path::new(&path).parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("napkin: could not create {}: {}", dir.display(), e);
            std::process::exit(1);
        }
    }
    if let Err(e) = std::fs::write(&path, &text) {
        eprintln!("napkin: could not write {}: {}", path, e);
        std::process::exit(1);
    }

    eprintln!("wrote {}", path);
    eprintln!();
    eprintln!("if this machine is not already in the dataset, open a pull request with that file.");
}

/// `(label, wants_all_cpus, runner)`. `wants_all_cpus` probes run unpinned.
type Runner<'a> = (&'static str, bool, Box<dyn FnOnce() -> Probe + 'a>);

fn probe_suite(m: &machine::Machine, seed: u64) -> Vec<Runner<'static>> {
    let ram = m.ram_bytes;
    vec![
        (
            "memory latency curve",
            false,
            Box::new(move || probes::memory::run(ram, seed)),
        ),
        ("syscall (getppid)", false, Box::new(probes::syscall::run)),
        ("uncontended mutex", false, Box::new(probes::mutex::run)),
        ("thread ping-pong rtt", true, Box::new(probes::thread::run)),
        (
            "branch mispredict",
            false,
            Box::new(move || probes::branch::run(seed)),
        ),
        (
            "page fault (first touch)",
            false,
            Box::new(probes::pagefault::run),
        ),
    ]
}

fn one_line(p: &Probe) -> String {
    if let Some(s) = &p.summary {
        format!("{:>9.2} {} (spread {:.1}%)", s.median, p.unit, s.spread_pct)
    } else if !p.points.is_empty() {
        let first = &p.points[0];
        let last = &p.points[p.points.len() - 1];
        format!(
            "{:>9.2} ns at {} -> {:.2} ns at {}",
            first.summary.median,
            human_bytes(first.x),
            last.summary.median,
            human_bytes(last.x)
        )
    } else {
        format!(
            "skipped: {}",
            p.note.as_deref().unwrap_or("no reason given")
        )
    }
}

fn human_bytes(b: u64) -> String {
    if b >= 1 << 20 {
        format!("{}M", b >> 20)
    } else {
        format!("{}K", b >> 10)
    }
}

fn report_invalid(g: &Guard) {
    eprintln!();
    eprintln!("this run is not valid:");
    for i in &g.invalidations {
        eprintln!("  - {}", i);
    }
}

fn build(
    m: &machine::Machine,
    g: &Guard,
    probes: &[Probe],
    seed: u64,
    pinned_cpu: Option<usize>,
) -> J {
    J::Obj(vec![
        ("schema", J::u(SCHEMA)),
        ("napkin_version", J::s(VERSION)),
        ("libc", J::s(LIBC)),
        (
            "run",
            J::Obj(vec![
                ("timestamp_utc", J::s(now_iso8601())),
                ("seed", J::u(seed)),
                (
                    "pinned_cpu",
                    pinned_cpu.map(|c| J::u(c as u64)).unwrap_or(J::Null),
                ),
                ("valid", J::Bool(g.valid())),
                (
                    "invalidations",
                    J::Arr(g.invalidations.iter().map(J::s).collect()),
                ),
                ("warnings", J::Arr(g.warnings.iter().map(J::s).collect())),
                ("guard", g.to_json()),
            ]),
        ),
        ("machine", m.to_json()),
        (
            "probes",
            J::Obj(
                probes
                    .iter()
                    .map(|p| (p.name, p.to_json()))
                    .collect::<Vec<_>>(),
            ),
        ),
    ])
}

fn parse_args() -> Result<Option<Args>, String> {
    let mut a = Args {
        out: None,
        stdout: false,
        seed: 0,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{}", HELP);
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("napkin {}", VERSION);
                return Ok(None);
            }
            "--stdout" => a.stdout = true,
            "--out" => {
                a.out = Some(it.next().ok_or("--out needs a path")?);
            }
            "--seed" => {
                let v = it.next().ok_or("--seed needs a number")?;
                a.seed = v.parse().map_err(|_| format!("bad --seed value: {}", v))?;
            }
            other => return Err(format!("unknown argument: {}", other)),
        }
    }
    if a.seed == 0 {
        // Recorded in the result file, so any run can be repeated exactly.
        a.seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x5EED)
            | 1;
    }
    Ok(Some(a))
}

const HELP: &str = "\
napkin — measured latency numbers for hardware that actually exists

usage: napkin [options]

  --out <path>   write the result file here
                 (default: results/<arch>/<cpu-slug>.json)
  --stdout       write the result to stdout instead of a file
  --seed <n>     seed the permutation rng (default: from the clock, recorded
                 in the result so a run can be repeated)
  -V, --version
  -h, --help

napkin refuses to produce a result if the machine is busy, the cpu governor is
not 'performance', or the machine slows down mid-run. that is deliberate: a
plausible-looking bad number is worse than no number.
";

/// ISO 8601 in UTC, to the second. Days-to-civil is Howard Hinnant's algorithm;
/// pulling in a date crate for one timestamp would break the one-dependency
/// rule for no benefit.
fn now_iso8601() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);

    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mth = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mth <= 2 { y + 1 } else { y };

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        mth,
        d,
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60
    )
}
