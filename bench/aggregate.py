#!/usr/bin/env python3
"""Aggregate the raw benchmark rows into honest statistics.

Reads results/raw.csv (written by run.sh), discards warm-up rows, and reports
median / p95 / stdev / n / failures per (tool, scenario, op) for three
metrics: wall-clock seconds, peak resident memory (max RSS), and CPU time.
Emits results/report.md and results/summary.json.

Failure handling: rows with rc != 0 are kept out of the stats, counted per
cell, and shown in the report as `[F failed of N]`. Any cell with at least
one failed row triggers a stderr line `FAILED tool scenario op: F of N` and
a non-zero exit code, unless --allow-failures is passed. The report and
summary.json are still written, so the evidence is kept.

Legacy aggregation: a raw.csv whose `tool` is the bare `docker-compose`
(older runs) is mapped to `docker-compose-podman` when results/engine was
`podman`, otherwise to `docker-compose-docker`. read_engine() stays for that.

raw.csv and summary.json stay in seconds at full precision, one canonical
unit. Only the report picks a readable one, per row (see row_unit).

No number is invented here: every statistic is computed from the measured
rows, and a losing result is printed exactly like a winning one.
"""
import csv
import json
import os
import statistics
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
RAW = os.path.join(HERE, "results", "raw.csv")
BUDGET = os.path.join(os.path.dirname(__file__), "memory-budget-mib")
MD = os.path.join(HERE, "results", "report.md")
JSON = os.path.join(HERE, "results", "summary.json")
ENGINE = os.path.join(HERE, "results", "engine")
ENV_TXT = os.path.join(HERE, "results", "env.txt")


def read_engine():
	"""Which engine docker-compose drove, as recorded by run.sh next to raw.csv.

	Empty when the file is absent: an older results directory, or a run where
	docker-compose was not measured at all.
	"""
	try:
		return open(ENGINE).read().strip()
	except OSError:
		return ""


def read_env():
	"""The environment file run.sh wrote, verbatim.

	Empty when env.txt is absent (older results directory). The report
	embeds it under "### Environment" so a reader can tell which podup,
	which Podman, which kernel and CPU governor produced the numbers below.
	"""
	try:
		return open(ENV_TXT).read()
	except OSError:
		return ""


def legacy_rename(tool, dc_engine):
	"""Map the legacy `docker-compose` tool to a current variant.

	Older runs wrote `docker-compose` and saved `results/engine`. New runs
	write the variant name directly. Picking the variant here lets the rest
	of the code have one path; an empty / `docker` engine means the run did
	docker-compose against the Docker daemon, which is `docker-compose-docker`
	in the new naming.
	"""
	if tool == "docker-compose":
		return "docker-compose-podman" if dc_engine == "podman" else "docker-compose-docker"
	return tool


def load(path):
	"""Read raw.csv, normalize legacy tool names, return all rows.

	Returns warm-up + failures too: the caller decides what to keep and what
	to count. Legacy `docker-compose` rows are renamed against the engine
	recorded at run time, so older results directories still aggregate.
	"""
	dc_engine = os.environ.get("BENCH_DOCKER_ENGINE", "") or read_engine()
	rows = list(csv.DictReader(open(path, newline="")))
	for r in rows:
		r["tool"] = legacy_rename(r["tool"], dc_engine)
	return rows

# Preferred ordering only. Anything measured but not listed here is appended
# rather than dropped: this list silently discarded four scenarios' worth of
# results (config-heavy, wide-running-ops, deep-chain, wide-level) because it was
# a filter, not an order: 972 rows measured, four scenarios never printed.
SCEN_ORDER = [
	"single", "multi-healthcheck", "deep-chain", "wide-level", "scale",
	"network-ipam", "volume-heavy", "secrets", "warm-restart", "many-services",
	"running-ops", "wide-running-ops", "config-heavy", "build",
]
OP_ORDER = ["up", "reup", "down", "config", "ps", "logs", "exec", "restart", "build"]
OP_LABEL = {
	"up": "up", "down": "down", "reup": "warm up", "ps": "ps", "logs": "logs",
	"exec": "exec", "restart": "restart", "build": "build", "config": "config",
}

# Tables: (tool_name_in_raw_csv, header_in_report_md).
# `docker-compose-podman` is the same-engine comparison; the variant name
# alone would mislead a reader who has only seen the legacy "docker-compose
# on Podman" wording.
PURE_COLS = [
	("podup", "podup"),
	("podman-compose", "podman-compose"),
	("docker-compose-podman", "docker-compose (Podman)"),
]
# Cross-engine: each tool shown on its native engine. Only printed when
# docker-compose-docker has rows, since the comparison needs all three to
# mean anything. The intro flags the rootless-vs-rootful caveat.
CROSS_COLS = [
	("podup", "podup"),
	("podman-compose", "podman-compose"),
	("docker-compose-docker", "docker-compose (Docker)"),
]

# Inline sample rows, shaped exactly like raw.csv, for `--self-test`. bench/results/
# is a local, git-ignored artifact directory (real numbers only come from the
# controlled, self-hosted benchmark run), so a shared-runner smoke check has no
# raw.csv to read. These rows exercise the same filtering and statistics path
# (warm-up discarded, failed rows counted, median/p95/stdev computed) without
# depending on committed benchmark data or a real Podman engine.
SELF_TEST_ROWS = [
	{"tool": "podup", "scenario": "single", "op": "up", "iter": "0", "phase": "warmup", "seconds": "0.520", "max_rss_kb": "10240", "cpu_s": "0.110", "rc": "0"},
	{"tool": "podup", "scenario": "single", "op": "up", "iter": "1", "phase": "measured", "seconds": "0.500", "max_rss_kb": "10000", "cpu_s": "0.100", "rc": "0"},
	{"tool": "podup", "scenario": "single", "op": "up", "iter": "2", "phase": "measured", "seconds": "0.510", "max_rss_kb": "10100", "cpu_s": "0.105", "rc": "0"},
	{"tool": "podup", "scenario": "single", "op": "down", "iter": "1", "phase": "measured", "seconds": "0.200", "max_rss_kb": "9500", "cpu_s": "0.050", "rc": "0"},
	{"tool": "podup", "scenario": "single", "op": "down", "iter": "2", "phase": "measured", "seconds": "0.210", "max_rss_kb": "9600", "cpu_s": "0.052", "rc": "0"},
	{"tool": "podman-compose", "scenario": "single", "op": "up", "iter": "1", "phase": "measured", "seconds": "0.800", "max_rss_kb": "30000", "cpu_s": "0.300", "rc": "0"},
	# The one failed row. The self-test asserts `[1 failed of 2]` shows up
	# in the rendered text and that the failure gate returns 1 over the
	# fixture and 0 when this row is dropped.
	{"tool": "podman-compose", "scenario": "single", "op": "up", "iter": "2", "phase": "measured", "seconds": "0.001", "max_rss_kb": "1", "cpu_s": "0.001", "rc": "1"},
	# Sub-10 ms rows, the ones /usr/bin/time could not see. They exercise the
	# millisecond branch of row_unit, which no whole-second fixture reaches.
	{"tool": "podup", "scenario": "running-ops", "op": "ps", "iter": "1", "phase": "measured", "seconds": "0.008521", "max_rss_kb": "9100", "cpu_s": "0.003812", "rc": "0"},
	{"tool": "podup", "scenario": "running-ops", "op": "ps", "iter": "2", "phase": "measured", "seconds": "0.008904", "max_rss_kb": "9150", "cpu_s": "0.003907", "rc": "0"},
	{"tool": "podman-compose", "scenario": "running-ops", "op": "ps", "iter": "1", "phase": "measured", "seconds": "0.412", "max_rss_kb": "29000", "cpu_s": "0.240", "rc": "0"},
	{"tool": "podman-compose", "scenario": "running-ops", "op": "ps", "iter": "2", "phase": "measured", "seconds": "0.421", "max_rss_kb": "29100", "cpu_s": "0.244", "rc": "0"},
	# Multi-second rows, so the seconds branch of row_unit is exercised too. A
	# fixture set that never crosses 1 s would leave half the formatting untested.
	{"tool": "podup", "scenario": "wide-level", "op": "up", "iter": "1", "phase": "measured", "seconds": "6.745", "max_rss_kb": "12800", "cpu_s": "1.040", "rc": "0"},
	{"tool": "podup", "scenario": "wide-level", "op": "up", "iter": "2", "phase": "measured", "seconds": "6.802", "max_rss_kb": "12900", "cpu_s": "1.061", "rc": "0"},
	{"tool": "podman-compose", "scenario": "wide-level", "op": "up", "iter": "1", "phase": "measured", "seconds": "41.220", "max_rss_kb": "61000", "cpu_s": "18.400", "rc": "0"},
	{"tool": "podman-compose", "scenario": "wide-level", "op": "up", "iter": "2", "phase": "measured", "seconds": "42.007", "max_rss_kb": "61200", "cpu_s": "18.910", "rc": "0"},
]


def row_unit(cells, metric):
	"""Pick one time unit for a whole report row, from its largest value.

	Returns (suffix, multiplier, decimals).

	One unit per row, applied to every tool in it. Choosing per cell would break
	the comparison the reader actually makes (one tool against another on the
	same operation) by putting "90 ms" next to "0.11 s". Across rows the
	workloads differ anyway, so a row is the widest scope where a shared unit
	still means something.

	p95 counts towards the choice, not just the median: a row whose median is a
	few milliseconds but whose p95 is over a second reads better in seconds than
	as a four-digit millisecond figure.
	"""
	values = [v for cell in cells if cell for v in (cell[metric]["median"], cell[metric]["p95"]) if v == v]
	if values and max(values) < 1.0:
		return ("ms", 1000.0, 1)
	# Above a minute, seconds stop being readable at a glance: a scenario that
	# takes two minutes printed as `120.000 s` makes the reader do the division.
	# No published row reaches this yet (the slowest is `wide-level up` at 9.7 s
	# for docker-compose) but the tier belongs here before a long scenario is
	# added rather than after, when the fix competes with reading the results.
	if values and max(values) >= 60.0:
		return ("min", 1.0 / 60.0, 2)
	return ("s", 1.0, 3)


def stats(values):
	"""Median, p95, stdev, n, min. Honest for small n.

	p95 is the nearest-rank percentile: with n=4 sorted [a, b, c, d] the rank
	formula picks d (the 4th of 4 sorted), so the value is the largest. That
	matches what a reader expects from "p95" without the noise of a normal-fit
	estimator on a tiny sample.
	"""
	if not values:
		return {"n": 0, "median": float("nan"), "p95": float("nan"), "stdev": 0.0, "min": float("nan")}
	s = sorted(values)
	k = max(0, min(len(s) - 1, round(0.95 * (len(s) - 1))))
	return {
		"n": len(values),
		"median": statistics.median(values),
		"p95": s[k],
		"stdev": statistics.pstdev(values) if len(values) > 1 else 0.0,
		"min": min(values),
	}


def check_failure_gate(rows):
	"""Return 1 if any measured row failed, else 0.

	The report and summary are written before this is called, so a caller
	that wants to keep them anyway can do so; this is a refuse-to-publish
	signal, not a write blocker. --allow-failures is checked at the call
	site so the self-test can verify both directions.
	"""
	return 1 if any(r["phase"] == "measured" and int(r["rc"]) != 0 for r in rows) else 0


# Table intros. One-liners (joined with a space) so the metric_table call stays
# one line and the count does not grow with each scenario added.
INTROS = {
	"pure_wall": "Lower is better. Median with p95 and stdev in parentheses. Each row carries one unit, picked from the largest value in it, so the tools in a row stay directly comparable; raw.csv and summary.json keep every figure in seconds. Identical engine, so the only difference is the compose tool. docker-compose appears here when it was pointed at the Podman socket rather than at a Docker daemon.",
	"pure_mem": "Peak resident memory (max RSS) and CPU time of the tool process per command, median. This is the client-side cost of running the tool: podup is a static binary talking to the Podman service, podman-compose is Python shelling out to `podman`.",
	"cross_wall": "podup and podman-compose drive rootless Podman, docker-compose drives the Docker daemon (rootful). This is what a user of each stack sees, and the engines differ, so it is not a pure tool comparison: docker-compose runs against dockerd, not the Podman socket, and engine differences are folded into its column.",
	"cross_mem": "Same caveat as the wall-clock table: podup and podman-compose run rootless, docker-compose runs rootful. Memory is the orchestrator process; engine-side work is not charged to any of them.",
}


def main():
	self_test = "--self-test" in sys.argv
	allow_failures = "--allow-failures" in sys.argv
	rows = SELF_TEST_ROWS if self_test else load(RAW)
	measured = [r for r in rows if r["phase"] == "measured"]
	successful = [r for r in measured if int(r["rc"]) == 0]
	tools = sorted({r["tool"] for r in measured})
	scen_set = {r["scenario"] for r in measured}
	# Ordered by preference, then anything else that was measured. A scenario
	# absent from SCEN_ORDER used to vanish from the report with no warning,
	# which is worse than an ugly order: the run costs half an hour and the
	# missing rows look like they were never measured.
	scenarios = [s for s in SCEN_ORDER if s in scen_set] + sorted(scen_set - set(SCEN_ORDER))

	# Per-cell counters of measured iterations and failures. Cells where every
	# measured row failed still get a row in the report (with `—` and the
	# failure count), so a failing scenario is visible instead of silently
	# dropped.
	failures, totals = {}, {}
	for r in measured:
		k = (r["tool"], r["scenario"], r["op"])
		totals[k] = totals.get(k, 0) + 1
		if int(r["rc"]) != 0:
			failures[k] = failures.get(k, 0) + 1

	# summary[tool][scenario][op] = {seconds:..., rss_mib:..., cpu_s:..., failed:F, measured:N}
	summary = {}
	for tool in tools:
		for scen in scenarios:
			for op in OP_ORDER:
				key = (tool, scen, op)
				if not totals.get(key):
					continue
				sel = [r for r in successful if r["tool"] == tool and r["scenario"] == scen and r["op"] == op]
				summary.setdefault(tool, {}).setdefault(scen, {})[op] = {
					"seconds": stats([float(r["seconds"]) for r in sel]),
					"rss_mib": stats([int(r["max_rss_kb"]) / 1024 for r in sel]),
					"cpu_s": stats([float(r["cpu_s"]) for r in sel]),
					"failed": failures.get(key, 0),
					"measured": totals.get(key, 0),
				}

	if not self_test:
		json.dump(summary, open(JSON, "w"), indent="\t", sort_keys=True)

	lines = []

	def metric_table(title, intro, cols, fmt):
		if not cols: return
		lines.append(f"### {title}\n")
		if intro: lines.append(intro + "\n")
		headers = [c[1] for c in cols]
		lines.append("| scenario | op | " + " | ".join(headers) + " |")
		lines.append("|" + "---|" * (len(headers) + 2))
		for scen in scenarios:
			for op in OP_ORDER:
				row = [summary.get(t, {}).get(scen, {}).get(op) for t, _ in cols]
				if not any(row): continue
				lines.append(f"| {scen} | {OP_LABEL[op]} | " + " | ".join(fmt(c, row) for c in row) + " |")
		lines.append("")

	def failure_suffix(cell):
		return f" [{cell['failed']} failed of {cell['measured']}]" if cell and cell.get("failed", 0) > 0 else ""

	def wall(cell, row):
		if not cell: return "—"
		s = cell["seconds"]; suf, mul, dec = row_unit(row, "seconds")
		return f"{s['median']*mul:.{dec}f} {suf} (p95 {s['p95']*mul:.{dec}f}, sd {s['stdev']*mul:.{dec}f}){failure_suffix(cell)}"

	def mem(cell, row):
		if not cell: return "—"
		# CPU time gets the same treatment as wall clock: rusage resolves to
		# microseconds, so a `ps` costing 4 ms of CPU no longer has to publish as
		# 0.004 s next to a build costing seconds.
		suf, mul, dec = row_unit(row, "cpu_s")
		r, c = cell["rss_mib"], cell["cpu_s"]
		return f"{r['median']:.1f} MiB / {c['median']*mul:.{dec}f} {suf}{failure_suffix(cell)}"

	# Embed the environment run.sh recorded, verbatim, so a reader can tell
	# which podup, which Podman, which kernel and CPU governor produced the
	# numbers below. Older results directories without env.txt skip this block.
	env_content = read_env()
	if env_content:
		lines.append("### Environment\n")
		lines.append("```")
		lines.append(env_content.rstrip("\n"))
		lines.append("```\n")

	lines.append("All numbers are over the measured iterations (warm-up discarded), same machine, same digest-pinned pre-pulled images, same compose file per scenario.\n")

	metric_table("Wall-clock, pure tool comparison (all drive Podman)", INTROS["pure_wall"], PURE_COLS, wall)
	metric_table("Memory + CPU, pure tool comparison (all drive Podman)", INTROS["pure_mem"], PURE_COLS, mem)

	# Cross-engine comparison: only meaningful when docker-compose-docker was
	# actually measured, otherwise the column would be empty and the row would
	# compare rootless Podman numbers against the void.
	if any(t == "docker-compose-docker" for t in tools):
		metric_table("Wall-clock, each tool on its own engine", INTROS["cross_wall"], CROSS_COLS, wall)
		metric_table("Memory + CPU, each tool on its own engine", INTROS["cross_mem"], CROSS_COLS, mem)
	else:
		lines.append("> docker-compose-docker was not measured on this host, so the cross-engine comparison is left blank rather than estimated.\n")

	if self_test:
		# The self-test runs on fixture rows. Writing them out would replace a
		# real report and summary, the output of a benchmark that takes the better
		# part of an hour and cannot be recomputed, since raw.csv is the only copy.
		# Printed, not just built: the fixtures exist to exercise the formatting,
		# and a table nobody looks at cannot show that a row picked the wrong
		# unit or that a cell came out empty.
		output_text = "\n".join(lines)
		print(output_text)
		# The unit tiers, exercised rather than assumed: a row is rendered in one
		# unit chosen from its largest value, and each boundary decides a report
		# column nobody re-reads once it is published.
		for value, want in ((0.5, "ms"), (1.0, "s"), (59.9, "s"), (60.0, "min"), (600.0, "min")):
			got = row_unit([{"seconds": {"median": value, "p95": value}}], "seconds")[0]
			if got != want:
				print(f"self-test FAILED: {value}s rendered in {got}, expected {want}", file=sys.stderr); return 1
		# Budget gate in both directions, like the failure gate below. The
		# fixture rows are synthetic, so they are not measured against the
		# real budget, but a gate nobody has watched fail is decoration.
		if check_memory_budget({"podup": {"single": {"up": {"rss_mib": {"median": 1.0}}}}}) != 0:
			print("self-test FAILED: the budget rejected a value under it", file=sys.stderr); return 1
		if check_memory_budget({"podup": {"single": {"up": {"rss_mib": {"median": 10_000.0}}}}}) != 1:
			print("self-test FAILED: the budget accepted a value over it", file=sys.stderr); return 1
		# Failure gate in both directions: the fixture has one failing
		# podman-compose row, so the rendered text must carry `[1 failed of 2]`
		# for that cell, and the gate must return 1. Removing the failing row
		# makes the gate return 0, like the memory budget check above.
		if "[1 failed of 2]" not in output_text:
			print("self-test FAILED: rendered text missing `[1 failed of 2]`", file=sys.stderr); return 1
		if check_failure_gate(SELF_TEST_ROWS) != 1:
			print("self-test FAILED: failure gate did not flag a row with rc != 0", file=sys.stderr); return 1
		if check_failure_gate([r for r in SELF_TEST_ROWS if int(r["rc"]) == 0]) != 0:
			print("self-test FAILED: failure gate flagged a row with no failures", file=sys.stderr); return 1
		print(f"self-test ok ({len(rows)} fixture rows); {MD} and {JSON} left untouched")
		return 0

	with open(MD, "w") as f:
		f.write("\n".join(lines))
	print(f"wrote {MD} and {JSON}")

	# Refuse to publish when any cell had a failed measured iteration: a median
	# over 2 successes out of 10 with nothing saying so is worse than no number
	# at all. The report and summary are already on disk, so the operator can
	# still see the failure count and decide.
	if check_failure_gate(measured) == 1:
		for t, s, o in sorted(failures):
			print(f"FAILED {t} {s} {o}: {failures[t, s, o]} of {totals[t, s, o]}", file=sys.stderr)
		if not allow_failures:
			return 1

	return check_memory_budget(summary)


def check_memory_budget(summary):
	"""Fail when podup's peak median RSS exceeds the budget in bench/memory-budget-mib.

	The releases standard asks for a size budget per artifact and treats an
	unexplained growth as a regression to investigate rather than ship. There was
	none for memory, so the drift from 7.5 MiB at 2.1.0 to 8.1 at 3.4.1 was
	noticed by a human reading a published table months later instead of by a red
	run on the day it landed.

	The budget is a file, not a literal here, for the same reason
	.github/podman-baseline-tests is: raising it is a reviewable commit that says
	someone decided to, not a number that drifts inside a script.
	"""
	if not os.path.exists(BUDGET):
		print(f"no memory budget at {BUDGET}; skipping the check", file=sys.stderr); return 0
	budget = float(open(BUDGET).read().strip())
	peaks = [(s, o, c["rss_mib"]["median"]) for s, ops in summary.get("podup", {}).items() for o, c in ops.items()]
	if not peaks:
		print("no podup rows to check against the memory budget", file=sys.stderr); return 0
	scen, op, worst = max(peaks, key=lambda t: t[2])
	print(f"memory: peak median {worst:.2f} MiB ({scen} {op}), budget {budget:.2f} MiB")
	if worst > budget:
		print(f"::error::podup peak median RSS {worst:.2f} MiB exceeds the {budget:.2f} MiB budget in bench/memory-budget-mib (worst: {scen} {op}). Attribute the growth, or raise the budget deliberately in its own commit.", file=sys.stderr)
		return 1
	return 0


if __name__ == "__main__":
	sys.exit(main())