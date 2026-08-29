#!/usr/bin/env python3
"""Validate result files before they enter the dataset.

Run with no arguments to check everything in results/, or pass paths.

This is what stands between the dataset and a plausible-looking bad number, so
it fails loudly and says which file and which field. It checks three things:
that the file is structurally a napkin result, that the run guard passed, and
that the values are physically possible. The last one exists because a
misconfigured or doctored submission does not announce itself.
"""

import json
import re
import sys
from pathlib import Path

SCHEMA = 1

# Fields that must never appear anywhere in a result. napkin does not collect
# them, so their presence means the file was hand-edited or produced by
# something else.
FORBIDDEN = re.compile(
    r"\b(hostname|username|user_name|mac_addr|macaddress|serial|serial_number|ip_addr|ipaddress)\b",
    re.I,
)

# (probe, field, low, high) in nanoseconds. Bounds are deliberately loose: they
# are here to catch a broken or fabricated submission, not to encode an opinion
# about what hardware is allowed to exist.
BOUNDS = [
    ("syscall_getppid", "median", 20.0, 50_000.0),
    ("mutex_lock_unlock_uncontended", "median", 2.0, 5_000.0),
    ("thread_pingpong_rtt", "median", 100.0, 5_000_000.0),
    ("branch_mispredict_penalty", "median", 0.0, 500.0),
    ("page_fault_first_touch", "median", 50.0, 500_000.0),
]

REQUIRED_MACHINE = ["arch", "os", "kernel", "cpu_model", "logical_cpus"]


def fail(path, msg, errors):
    errors.append(f"{path}: {msg}")


def check(path, errors, seen):
    raw = path.read_text()
    if FORBIDDEN.search(raw):
        fail(path, "contains a field napkin never collects; was this hand-edited?", errors)

    try:
        d = json.loads(raw)
    except json.JSONDecodeError as e:
        fail(path, f"not valid JSON: {e}", errors)
        return

    if d.get("schema") != SCHEMA:
        fail(path, f"schema is {d.get('schema')!r}, expected {SCHEMA}", errors)
        return

    for key in ("napkin_version", "run", "machine", "probes"):
        if key not in d:
            fail(path, f"missing top-level key {key!r}", errors)
            return

    run, machine, probes = d["run"], d["machine"], d["probes"]

    if run.get("valid") is not True:
        why = "; ".join(run.get("invalidations") or ["no reason recorded"])
        fail(path, f"run is marked invalid: {why}", errors)

    for key in REQUIRED_MACHINE:
        if not machine.get(key):
            fail(path, f"machine.{key} is missing or empty", errors)
    if machine.get("cpu_model") == "unknown":
        fail(path, "cpu_model is 'unknown'; the dataset is indexed by cpu, so this cannot be filed", errors)

    # The path is the dataset's index, so it has to agree with the contents.
    arch = machine.get("arch")
    if arch and path.parent.name != arch:
        fail(path, f"is under results/{path.parent.name}/ but machine.arch is {arch!r}", errors)

    key = (machine.get("cpu_model"), machine.get("logical_cpus"), machine.get("kernel"))
    if key in seen:
        fail(path, f"duplicates {seen[key].name} (same cpu, cpu count and kernel)", errors)
    else:
        seen[key] = path

    for name, field, low, high in BOUNDS:
        p = probes.get(name)
        if p is None:
            fail(path, f"missing probe {name!r}", errors)
            continue
        if not p.get("measured"):
            continue  # legitimately skipped on this platform, and says so
        v = p.get(field)
        if not isinstance(v, (int, float)):
            fail(path, f"{name}.{field} is {v!r}, not a number", errors)
        elif not (low <= v <= high):
            fail(path, f"{name}.{field} is {v} ns, outside the plausible range {low}-{high} ns", errors)

    curve = probes.get("memory_latency_curve", {}).get("curve")
    if not curve:
        fail(path, "memory_latency_curve has no points; it is the whole point of the dataset", errors)
    elif len(curve) < 6:
        fail(path, f"memory_latency_curve has only {len(curve)} points, expected at least 6", errors)
    else:
        first, last = curve[0]["median"], curve[-1]["median"]
        if last <= first:
            fail(
                path,
                f"memory latency does not rise with working set ({first} ns at the smallest, "
                f"{last} ns at the largest); the pointer chase was probably optimized out",
                errors,
            )
        if first < 0.1:
            fail(path, f"smallest working set reads {first} ns, which is below one cpu cycle", errors)


def main(argv):
    paths = [Path(a) for a in argv[1:]] or sorted(Path("results").rglob("*.json"))
    if not paths:
        print("no result files found", file=sys.stderr)
        return 1

    errors, seen = [], {}
    for p in paths:
        check(p, errors, seen)

    for e in errors:
        print(f"error: {e}", file=sys.stderr)
    print(f"checked {len(paths)} result file(s), {len(errors)} error(s)", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
