# Result file schema (`schema: 1`)

One file per machine, at `results/<arch>/<cpu-slug>-<n>cpu.json`. The path is
the dataset's index, so `machine.arch` must agree with the directory.

All times are **nanoseconds**. All sizes are **bytes**.

## Top level

| field | meaning |
|---|---|
| `schema` | Format version. Currently `1`. |
| `napkin_version` | Version of the binary that produced the file. |
| `run` | Conditions the measurement was taken under. |
| `machine` | What the hardware is. |
| `probes` | The measurements. |

## `run`

| field | meaning |
|---|---|
| `timestamp_utc` | ISO 8601, to the second. |
| `seed` | RNG seed for the pointer-chase permutation. Pass it back with `--seed` to reproduce the identical memory layout (timings will still vary run to run). |
| `pinned_cpu` | Logical CPU the single-threaded probes were pinned to, or `null` if pinning was unavailable. |
| `valid` | **`false` means do not use this file.** Files with `valid: false` are never committed; the binary refuses to write one. |
| `invalidations` | Why the run failed, if it did. Non-empty implies `valid: false`. |
| `warnings` | Conditions that degrade the result without invalidating it — virtualization, an unknown governor, a non-Linux platform. |
| `guard.calibration_start_ns` / `_end_ns` | A fixed CPU-bound workload timed at the start and end of the run. |
| `guard.calibration_drift_pct` | How much slower the machine got during the run. Past `max_drift_pct` the run is invalid; on a laptop this is thermal throttling. |

## `machine`

`arch`, `os`, `kernel`, `cpu_model`, `logical_cpus`, `physical_cores`,
`ram_bytes`, `governor`, `virtualized`, and `caches` — a list of
`{level, type, bytes}` from sysfs, where `type` is `Data`, `Instruction` or
`Unified`.

Deliberately absent: hostname, username, MAC address, serial numbers, IP
addresses. napkin never collects them, and `tools/validate.py` rejects any file
in which they appear.

Fields that could not be determined on this platform are `null` rather than
guessed.

## `probes`

Each probe is keyed by name. Every one carries:

| field | meaning |
|---|---|
| `unit` | Always `ns` at present. |
| `what` | One sentence stating exactly what was measured. Read this before quoting a number. |
| `measured` | `false` if the probe was skipped on this platform; `note` says why. |
| `stable` | `false` if the samples spread past 20% of the median. |

Scalar probes add `median`, `min`, `p10`, `p90`, `spread_pct`, `samples`.
`spread_pct` is the **interquartile** range over the median, not p10–p90: with
a couple of dozen samples the outer percentiles sit close to the extremes, so a
single preempted sample would read as a broken probe.

`memory_latency_curve` instead carries `curve`, a list of points with
`x` (working-set bytes), `median`, `min`, `spread_pct` and `stable`, plus
`unstable_x` listing the flagged working sets. Expect the points at a cache's
capacity to be flagged — that region is genuinely bimodal. Use `min` for the
latency of the cache itself and `median` for what the machine does in practice.

### The probes

| name | what |
|---|---|
| `memory_latency_curve` | One dependent load, chasing a random 64-byte-strided pointer cycle, swept across working-set sizes. |
| `syscall_getppid` | One `getppid(2)` round trip into the kernel. Not `getpid`, which some libcs cache or serve from the vDSO. |
| `mutex_lock_unlock_uncontended` | One lock/unlock pair on an uncontended mutex, including the guarded increment. |
| `thread_pingpong_rtt` | A send-and-receive round trip between two threads over a zero-capacity rendezvous channel. Two hand-offs plus channel overhead — **not** a bare context switch. |
| `branch_mispredict_penalty` | Per-element cost by which a shuffled walk beats a sorted walk over identical data. About half the elements are mispredictable, so the per-miss penalty is near twice this. |
| `page_fault_first_touch` | One minor fault: the first write to a page of a fresh anonymous mapping, including the kernel zeroing it. |
