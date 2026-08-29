# napkin

**Latency numbers for hardware that actually exists.**

The table everyone quotes in design reviews — "L1 is 0.5ns, a disk seek is
10ms" — was assembled around 2012 and has been copied by hand ever since. Some
of it is still fine. Some of it is wrong by two orders of magnitude. Nobody is
maintaining it, because maintaining it would mean owning a hundred machines.

So this is the other way round: **one static binary, one JSON file, one pull
request.** You run napkin on your machine, you commit the file it writes, and
your hardware is in the dataset with your name on the commit.

```
$ napkin
napkin 0.1.0 — Intel(R) Core(TM) i3-2350M CPU @ 2.30GHz
  4 logical cpus / 2 cores, x86_64 kernel 6.8.0-138-generic

  memory latency curve                 2.18 ns at 4K -> 102.81 ns at 128M
  syscall (getppid)                  612.34 ns (spread 1.8%)
  uncontended mutex                   25.69 ns (spread 0.9%)
  thread ping-pong rtt             11553.58 ns (spread 5.3%)
  branch mispredict                    5.06 ns (spread 1.8%)
  page fault (first touch)          2403.11 ns (spread 6.9%)

wrote results/x86_64/intel-r-core-tm-i3-2350m-cpu-2-30ghz-4cpu.json
```

## Contributing a machine

```sh
git clone <this repo> && cd napkin
cargo build --release

# linux: probes are unreproducible on a ramping clock, so pin it first.
# macos has no equivalent and needs no step here — napkin notes it in the file.
sudo cpupower frequency-set -g performance

./target/release/napkin
python3 tools/validate.py
```

Then open a pull request containing exactly the one file it wrote. That is the
entire contribution. You do not need to read the Rust.

**Interesting hardware is worth more than fast hardware.** A Raspberry Pi, a
2011 ThinkPad, an Ampere Altra, a POWER9 workstation, a RISC-V board, anything
running under a hypervisor — all of these tell you something a fourteenth
recent x86 laptop does not. If your machine is not in `results/`, it is wanted.

## napkin refuses more than it reports

A wrong number that looks plausible is worse than no number, because once it is
committed nobody can tell it apart from a good one. So the run guard aborts
rather than guesses. It will refuse to write a file if:

- the CPU governor is not `performance` — a ramping clock is not reproducible;
- the 1-minute load average is above 30% of your core count — something else
  was running, and you measured it too;
- the machine got more than 5% slower between the start and end of the run.
  That last one is a calibration workload timed twice, and on a laptop it
  almost always means thermal throttling.

Running under a hypervisor is normally a warning, not a refusal — cloud
instances are the hardware most software actually runs on, and those numbers are
real, they just include the host. But a guest that exposes **no** cpufreq
governor is refused: there is then no way to confirm the clock was pinned, and a
result whose timing environment is entirely unverifiable cannot honestly be
attributed to the CPU it names. That combination is what a shared CI runner
looks like.

Within a run, each probe reports a median and an interquartile spread, and any
probe that spreads past 20% is flagged `"stable": false` in the output rather
than quietly averaged.

## Reading the memory curve

The curve is the centrepiece: a pointer chase around a single random cycle of
64-byte-strided slots, so every load depends on the one before it and the
prefetcher has nothing to work with. Sweeping the working set from 4KiB to
128MiB makes the machine's cache hierarchy fall out of the data. On the i3-2350M
above, against a reported 32K L1d / 256K L2 / 3072K L3:

| working set | ns | what you are looking at |
|---|---|---|
| 4K – 32K | 2.2 | L1, 5 cycles at 2.3GHz |
| 64K – 128K | 5.3 | L2, 12 cycles |
| 512K – 1M | 14.4 | L3, 33 cycles |
| 2M | 20 – 29, **bimodal** | at L3 capacity — see below |
| 4M – 128M | 83 – 103 | DRAM |

**Points at a cache's capacity spread wide, on every machine, every run.** At
2M against a 3M L3 the working set neither fits nor doesn't: part stays
resident, part is evicted between samples, and the distribution is genuinely
bimodal. That is a real property of the hardware, so napkin flags those points
and leaves the rest of the curve standing rather than condemning all fifteen.
`tools/table.py` reads each cache level at half its capacity, well clear of it.

## What is not measured yet

Stated plainly, because the gaps are the roadmap:

- **Disk.** 4K random read is the single most-wrongly-quoted number in the 2012
  table, and it needs `O_DIRECT` to mean anything at all — otherwise you
  measure the page cache. Not implemented rather than implemented badly.
- **Network RTT.** Needs a fixed public endpoint, which is an operational
  commitment and a privacy question, not just code.
- **CPU pinning on macOS.** Linux and macOS are both supported; every probe
  runs and every machine fact resolves. But macOS exposes only affinity
  *hints*, which the scheduler may ignore, so probes there run unpinned and the
  result says so. Expect a slightly wider spread than the same silicon
  under Linux.
- **Windows and the BSDs.** The binary should build, but machine facts come
  back `unknown` and `tools/validate.py` will reject the result, because the
  dataset is indexed by CPU. Adding a platform means adding one `facts` module
  in `src/machine.rs`.
- **Core pinning for the thread probe.** It runs unpinned, so scheduler
  placement is part of what it measures. Pinning the two threads to two known
  physical cores would make it comparable across machines.

## Layout

```
src/probes/     one file per probe, each documenting its own method
src/guard.rs    the refusals
tools/validate.py   what CI runs on every pull request
tools/table.py      renders the canonical table from results/
results/<arch>/     the dataset
```

## License

MIT.
