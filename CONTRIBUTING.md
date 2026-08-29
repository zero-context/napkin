# Contributing

## Adding your machine

This is the common case and it should take about five minutes.

1. `cargo build --release`
2. `sudo cpupower frequency-set -g performance` (or your distro's equivalent —
   napkin refuses to run otherwise, and explains why)
3. Close what you can. A browser is enough to fail the load check.
4. `./target/release/napkin`
5. `python3 tools/validate.py`
6. Commit **only** the one JSON file under `results/` and open a pull request.

If the run refuses, the message says which condition failed and what to do
about it. A refusal is the tool working.

### What gets merged

- The run guard passed (`"valid": true`).
- `tools/validate.py` is clean.
- The machine is not already in the dataset, or the existing entry is on an
  older kernel and yours is worth having alongside it.

We are not filtering on how fast your machine is. An old or slow or unusual
machine is a more useful contribution than another current laptop.

### What is in the file

No hostname, no username, no MAC address, no serial number, no IP. napkin does
not collect them, and `tools/validate.py` rejects any result file that contains
them — which would mean it had been hand-edited. What is collected: CPU model,
core count, RAM size, kernel version, cache topology, governor, and whether you
are virtualized. If you would rather check before committing, the file is
plain JSON and every field is documented in `SCHEMA.md`.

## Working on the code

Good first changes, roughly in order of how self-contained they are:

- **Write the `what` string for a probe more clearly.** Every probe carries a
  one-sentence description into the result file. If one is vague, that is a
  real bug — people read those instead of the source.
- **Port a probe to another platform.** `page_fault_first_touch` and the CPU
  pinning in `machine.rs` are Linux-only and currently skip elsewhere with a
  note. macOS and the BSDs need `mach_vm_allocate` and
  `thread_policy_set` equivalents.
- **Machine facts on non-x86.** `cpu_model` falls back through several
  `/proc/cpuinfo` keys and then the device tree. If it returns `unknown` on
  your board, that is a one-line fix and the dataset cannot index your machine
  without it.
- **Add a probe.** See below.

### Adding a probe

One file in `src/probes/`, one entry in `probe_suite()` in `main.rs`. The rules
that matter:

- **Name it after what you measure, not the concept it approximates.** The
  thread probe is `thread_pingpong_rtt`, not `context_switch`, because a round
  trip over a rendezvous channel is two switches plus the channel. Overclaiming
  in a name is how a dataset stops being trusted.
- **Return a median and a spread**, via `probes::bench`. Never a mean: one
  descheduled sample moves a mean by an order of magnitude.
- **Defeat the optimizer honestly.** Prefer a genuine data dependency to
  sprinkling `black_box`, which costs cycles on every iteration — that alone
  was inflating the L1 measurement from 5 cycles to 10.
- **Do not swallow errors.** A failed `mmap` must not be averaged in as a very
  fast page fault. Panic with the OS error; a loud failure is recoverable and a
  silent one poisons the dataset.
- **Set the plausible bounds** for your probe in `tools/validate.py`. Loose is
  fine — they exist to catch a broken submission, not to legislate what
  hardware may exist.

### Dependencies

napkin depends on `libc` and nothing else, and that is a hard rule rather than
a preference. This binary needs to build on a RISC-V single-board computer and
a POWER9 workstation owned by someone who is doing us a favour. Every crate
added is a machine that silently drops out of the dataset. The JSON writer in
`src/json.rs` is forty lines for exactly this reason.
