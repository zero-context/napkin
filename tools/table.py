#!/usr/bin/env python3
"""Render the canonical latency table from results/.

Writes Markdown to stdout, or splices it into a file between
`<!-- BEGIN:dataset -->` and `<!-- END:dataset -->` with `--inject <file>`. This is the artifact the project exists to produce:
the numbers everyone quotes, for hardware that is actually in service, with the
machine each one came from named next to it.

Cache-level latencies are read off the memory curve using the cache sizes the
machine reports, sampling at half of each level's capacity — comfortably inside
it, and clear of the bimodal region at the capacity boundary.
"""

import json
import sys
from pathlib import Path


def load(root):
    out = []
    for p in sorted(Path(root).rglob("*.json")):
        d = json.loads(p.read_text())
        if d.get("run", {}).get("valid"):
            out.append(d)
    return out


def largest_stable_at_or_below(curve, ceiling_bytes):
    """The largest stable curve point whose working set fits under a ceiling.

    Not "the nearest point": nearest-by-distance will happily pick a point just
    above the ceiling, and the points just above a cache's capacity are the
    bimodal ones the probe flags as unstable. Staying at or below the ceiling
    and skipping flagged points keeps the reported figure inside the level it
    claims to describe.
    """
    fits = [pt for pt in curve if pt["x"] <= ceiling_bytes and pt.get("stable", True)]
    return max(fits, key=lambda pt: pt["x"]) if fits else None


def cache_latencies(d):
    """{level: ns} for each data-holding cache, plus 'DRAM'."""
    curve = d["probes"].get("memory_latency_curve", {}).get("curve") or []
    if not curve:
        return {}
    caches = [c for c in d["machine"].get("caches", []) if c["type"] in ("Data", "Unified")]
    caches.sort(key=lambda c: c["level"])

    out = {}
    for c in caches:
        # Half of capacity, so the working set is comfortably inside the level
        # and clear of the bimodal region at its boundary.
        pt = largest_stable_at_or_below(curve, c["bytes"] / 2)
        # `min` rather than `median`: the fastest sample is the one where the
        # working set was actually resident, which is the latency of the cache
        # itself rather than of this machine's background noise.
        if pt:
            out[f"L{c['level']}"] = pt["min"]
    dram = [pt for pt in curve if pt.get("stable", True)]
    if dram:
        out["DRAM"] = max(dram, key=lambda pt: pt["x"])["min"]
    return out


def probe(d, name, field="median"):
    p = d["probes"].get(name)
    if not p or not p.get("measured"):
        return None
    return p.get(field)


def fmt(v, unit="ns"):
    if v is None:
        return "—"
    if unit == "ns" and v >= 10_000:
        return f"{v/1000:,.1f} µs"
    if v >= 100:
        return f"{v:,.0f} ns"
    return f"{v:.2f} ns"


BEGIN = "<!-- BEGIN:dataset -->"
END = "<!-- END:dataset -->"


def render(results, embed=False):
    """Build the markdown. `embed` drops the document title, since the file it
    is spliced into already has one."""
    out = []
    if not embed:
        out.append("# Latency numbers, measured\n")
    out.append(
        f"{len(results)} machine(s) in the dataset. Every row is a real measurement from a "
        "real machine that passed the run guard, not an estimate.\n"
    )

    cols = ["L1", "L2", "L3", "DRAM"]
    out.append("### Memory hierarchy\n")
    out.append("| Machine | Arch | " + " | ".join(cols) + " |")
    out.append("|---|---|" + "---|" * len(cols))
    for d in results:
        lat = cache_latencies(d)
        cells = [fmt(lat.get(c)) for c in cols]
        out.append(
            f"| {d['machine']['cpu_model']} | {d['machine']['arch']} | " + " | ".join(cells) + " |"
        )

    out.append("\n### Everything else\n")
    out.append("| Machine | Syscall | Mutex | Page fault | Branch miss | Thread RTT |")
    out.append("|---|---|---|---|---|---|")
    for d in results:
        out.append(
            "| "
            + " | ".join(
                [
                    d["machine"]["cpu_model"],
                    fmt(probe(d, "syscall_getppid")),
                    fmt(probe(d, "mutex_lock_unlock_uncontended")),
                    fmt(probe(d, "page_fault_first_touch")),
                    fmt(probe(d, "branch_mispredict_penalty")),
                    fmt(probe(d, "thread_pingpong_rtt")),
                ]
            )
            + " |"
        )

    out.append(
        "\n*Branch miss is the per-element cost of an unpredictable branch; roughly half the "
        "elements are mispredictable, so the per-misprediction penalty is near twice it. "
        "Thread RTT is a round trip over a rendezvous channel, not a bare context switch. "
        "Cache levels are read at half of each level's capacity, clear of the bimodal region "
        "at the boundary.*"
    )
    return "\n".join(out) + "\n"


def inject(path, body):
    """Splice the table between the markers in `path`. Returns True if the file
    changed, so a caller can skip an empty commit."""
    text = Path(path).read_text()
    if BEGIN not in text or END not in text:
        raise SystemExit(f"{path}: missing {BEGIN} / {END} markers")
    head, rest = text.split(BEGIN, 1)
    _, tail = rest.split(END, 1)
    updated = f"{head}{BEGIN}\n{body}{END}{tail}"
    if updated == text:
        return False
    Path(path).write_text(updated)
    return True


def main(argv):
    args = argv[1:]
    target = None
    if "--inject" in args:
        i = args.index("--inject")
        try:
            target = args[i + 1]
        except IndexError:
            raise SystemExit("--inject needs a file path")
        args = args[:i] + args[i + 2 :]

    results = load(args[0] if args else "results")
    if not results:
        print("no valid results found", file=sys.stderr)
        return 1

    if target:
        changed = inject(target, render(results, embed=True))
        print(
            f"{target}: {'updated' if changed else 'already current'} "
            f"({len(results)} machine(s))",
            file=sys.stderr,
        )
        return 0

    print(render(results), end="")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
