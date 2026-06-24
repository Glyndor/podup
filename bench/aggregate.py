#!/usr/bin/env python3
"""Aggregate the raw benchmark rows into honest statistics.

Reads results/raw.csv (written by run.sh), discards warm-up rows and any row
whose command failed (rc != 0), and reports median / p95 / stdev / n per
(tool, scenario, op) for three metrics: wall-clock seconds, peak resident memory
(max RSS), and CPU time. Emits results/report.md and results/summary.json.

No number is invented here: every statistic is computed from the measured rows,
and a losing result is printed exactly like a winning one.
"""
import csv
import json
import os
import statistics
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
RAW = os.path.join(HERE, "results", "raw.csv")
MD = os.path.join(HERE, "results", "report.md")
JSON = os.path.join(HERE, "results", "summary.json")

SCEN_ORDER = [
	"single", "multi-healthcheck", "scale", "network-ipam", "volume-heavy",
	"warm-restart", "many-services", "running-ops", "build",
]
OP_ORDER = ["up", "reup", "down", "ps", "logs", "exec", "restart", "build"]
OP_LABEL = {
	"up": "up", "down": "down", "reup": "warm up", "ps": "ps", "logs": "logs",
	"exec": "exec", "restart": "restart", "build": "build",
}


def pct(values, p):
	"""Nearest-rank percentile; honest for small n."""
	if not values:
		return float("nan")
	s = sorted(values)
	k = max(0, min(len(s) - 1, round(p / 100 * (len(s) - 1))))
	return s[k]


def stats(values):
	return {
		"n": len(values),
		"median": statistics.median(values) if values else float("nan"),
		"p95": pct(values, 95),
		"stdev": statistics.pstdev(values) if len(values) > 1 else 0.0,
		"min": min(values) if values else float("nan"),
	}


def load(path):
	rows = []
	with open(path, newline="") as f:
		for r in csv.DictReader(f):
			if r["phase"] != "measured" or int(r["rc"]) != 0:
				continue
			rows.append(r)
	return rows


def main():
	if not os.path.exists(RAW):
		print(f"no raw data at {RAW}", file=sys.stderr)
		return 1
	rows = load(RAW)
	tools = sorted({r["tool"] for r in rows})
	scenarios = [s for s in SCEN_ORDER if any(r["scenario"] == s for r in rows)]

	# summary[tool][scenario][op] = {seconds:..., rss_mib:..., cpu_s:...}
	summary = {}
	for tool in tools:
		for scen in scenarios:
			for op in OP_ORDER:
				sel = [r for r in rows if r["tool"] == tool
					   and r["scenario"] == scen and r["op"] == op]
				if not sel:
					continue
				cell = {
					"seconds": stats([float(r["seconds"]) for r in sel]),
					"rss_mib": stats([int(r["max_rss_kb"]) / 1024 for r in sel]),
					"cpu_s": stats([float(r["cpu_s"]) for r in sel]),
				}
				summary.setdefault(tool, {}).setdefault(scen, {})[op] = cell

	with open(JSON, "w") as f:
		json.dump(summary, f, indent="\t", sort_keys=True)

	same = [t for t in tools if t in ("podup", "podman-compose")]
	cross = [t for t in tools if t == "docker-compose"]
	lines = []

	def metric_table(title, intro, cols, fmt):
		if not cols:
			return
		lines.append(f"### {title}\n")
		if intro:
			lines.append(intro + "\n")
		lines.append("| scenario | op | " + " | ".join(cols) + " |")
		lines.append("|" + "---|" * (len(cols) + 2))
		for scen in scenarios:
			for op in OP_ORDER:
				if not any(op in summary.get(c, {}).get(scen, {}) for c in cols):
					continue
				cells = [fmt(summary.get(c, {}).get(scen, {}).get(op)) for c in cols]
				lines.append(f"| {scen} | {OP_LABEL[op]} | " + " | ".join(cells) + " |")
		lines.append("")

	def wall(cell):
		if not cell:
			return "—"
		s = cell["seconds"]
		return f"{s['median']:.3f} (p95 {s['p95']:.3f}, sd {s['stdev']:.3f})"

	def mem(cell):
		if not cell:
			return "—"
		r, c = cell["rss_mib"], cell["cpu_s"]
		return f"{r['median']:.1f} MiB / {c['median']:.3f} s"

	lines.append("All numbers are over the measured iterations (warm-up "
				 "discarded), same machine, same digest-pinned pre-pulled images, "
				 "same compose file per scenario.\n")

	metric_table(
		"Wall-clock — pure tool comparison (both drive Podman)",
		"Seconds, lower is better. Median with p95 and stdev in parentheses. "
		"Identical engine, only the compose tool differs.",
		same, wall)
	metric_table(
		"Memory + CPU — pure tool comparison (both drive Podman)",
		"Peak resident memory (max RSS) and CPU time of the tool process per "
		"command, median. This is the client-side cost of running the tool: "
		"podup is a static binary talking to the Podman service, podman-compose "
		"is Python shelling out to `podman`.",
		same, mem)
	metric_table(
		"Wall-clock — cross-engine stack (different daemon)",
		"docker-compose drives dockerd, so these are an end-to-end stack "
		"comparison, not pure-tool. Only present when a Docker Engine was "
		"available on the benchmark host.",
		cross, wall)
	if not cross:
		lines.append("> docker-compose was not measured on this host (no Docker "
					 "Engine available); the cross-engine comparison is left blank "
					 "rather than estimated.\n")

	with open(MD, "w") as f:
		f.write("\n".join(lines))
	print(f"wrote {MD} and {JSON}")
	return 0


if __name__ == "__main__":
	sys.exit(main())
