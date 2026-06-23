#!/usr/bin/env python3
"""Aggregate the raw benchmark rows into honest statistics.

Reads results/raw.csv (written by run.sh), discards warm-up rows and any row
whose command failed (rc != 0), and reports median / p95 / stdev / n per
(tool, scenario, phase). Emits results/report.md and results/summary.json.

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
	"single", "multi-healthcheck", "scale", "network-ipam",
	"volume-heavy", "warm-restart", "many-services",
]
OP_LABEL = {"up": "up", "down": "down", "reup": "warm up"}


def pct(values, p):
	"""Nearest-rank percentile; honest for small n."""
	if not values:
		return float("nan")
	s = sorted(values)
	k = max(0, min(len(s) - 1, round(p / 100 * (len(s) - 1))))
	return s[k]


def load(path):
	rows = []
	with open(path, newline="") as f:
		for r in csv.DictReader(f):
			if r["phase"] != "measured":
				continue
			if int(r["rc"]) != 0:
				continue
			rows.append(r)
	return rows


def stats(values):
	return {
		"n": len(values),
		"median": statistics.median(values) if values else float("nan"),
		"p95": pct(values, 95),
		"stdev": statistics.pstdev(values) if len(values) > 1 else 0.0,
		"min": min(values) if values else float("nan"),
	}


def main():
	if not os.path.exists(RAW):
		print(f"no raw data at {RAW}", file=sys.stderr)
		return 1
	rows = load(RAW)
	tools = sorted({r["tool"] for r in rows})
	scenarios = [s for s in SCEN_ORDER if any(r["scenario"] == s for r in rows)]

	summary = {}
	for tool in tools:
		summary[tool] = {}
		for scen in scenarios:
			for op in ("up", "down", "reup"):
				vals = [float(r["seconds"]) for r in rows
						if r["tool"] == tool and r["scenario"] == scen and r["op"] == op]
				if vals:
					summary.setdefault(tool, {}).setdefault(scen, {})[op] = stats(vals)

	with open(JSON, "w") as f:
		json.dump(summary, f, indent="\t", sort_keys=True)

	same_engine = [t for t in tools if t in ("podup", "podman-compose")]
	cross = [t for t in tools if t == "docker-compose"]

	lines = []
	lines.append("Wall-clock, lower is better. Each cell is **median** with **p95** "
				 "and **stdev** in parentheses, in seconds, over the measured "
				 "iterations (warm-up discarded). Same machine, same digest-pinned "
				 "pre-pulled images, same compose file per scenario.\n")

	def table(title, cols, note):
		if not cols:
			return
		lines.append(f"### {title}\n")
		if note:
			lines.append(note + "\n")
		head = "| scenario | op | " + " | ".join(cols) + " |"
		sep = "|" + "---|" * (len(cols) + 2)
		lines.append(head)
		lines.append(sep)
		for scen in scenarios:
			for op in ("up", "reup", "down"):
				if not any(op in summary.get(c, {}).get(scen, {}) for c in cols):
					continue
				cells = []
				for c in cols:
					s = summary.get(c, {}).get(scen, {}).get(op)
					if s:
						cells.append(f"{s['median']:.3f} (p95 {s['p95']:.3f}, sd {s['stdev']:.3f})")
					else:
						cells.append("—")
				lines.append(f"| {scen} | {OP_LABEL[op]} | " + " | ".join(cells) + " |")
		lines.append("")

	table("Same-engine — pure tool comparison (both drive Podman)", same_engine,
		  "This is the apples-to-apples comparison: identical engine, only the "
		  "compose tool differs.")
	table("Cross-engine — end-to-end stack (different daemon)", cross,
		  "docker-compose drives dockerd, a different daemon, so these numbers are "
		  "a whole-stack comparison, not a pure-tool one. Only present when a "
		  "Docker Engine was available on the benchmark host.")
	if not cross:
		lines.append("> docker-compose was not measured on this host (no Docker "
					 "Engine available). The cross-engine comparison requires a "
					 "running dockerd; it is intentionally left blank rather than "
					 "estimated.\n")

	with open(MD, "w") as f:
		f.write("\n".join(lines))
	print(f"wrote {MD} and {JSON}")
	return 0


if __name__ == "__main__":
	sys.exit(main())
