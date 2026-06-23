#!/usr/bin/env bash
# Fair compose-tool benchmark runner.
#
# Drives each tool through the same scenario suite, the same number of times,
# on the same machine, against digest-pinned, pre-pulled images. It measures
# wall-clock only and writes one raw row per timed run to results/raw.csv; the
# statistics (median / p95 / stdev) are computed by aggregate.py, never here.
#
# Fairness is the whole point: identical compose inputs, identical lifecycle,
# warm-up iterations discarded, every scenario reported. Same-engine tools
# (podup, podman-compose) drive Podman and are a pure tool comparison;
# docker-compose drives dockerd and is only run when a Docker daemon is present,
# always labelled as an end-to-end stack comparison.
#
# Usage: bench/run.sh [--iters N] [--warmup W] [--cores CPUSET] [--smoke]
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCEN_DIR="$HERE/scenarios"
OUT_DIR="$HERE/results"
RAW="$OUT_DIR/raw.csv"

ITERS=12
WARMUP=2
CORES=""        # e.g. "0-3"; empty = no pinning
SMOKE=0
PODUP_BIN="${PODUP_BIN:-podup}"

while [ $# -gt 0 ]; do
	case "$1" in
		--iters) ITERS="$2"; shift 2 ;;
		--warmup) WARMUP="$2"; shift 2 ;;
		--cores) CORES="$2"; shift 2 ;;
		--smoke) SMOKE=1; ITERS=1; WARMUP=0; shift ;;
		*) echo "unknown arg: $1" >&2; exit 2 ;;
	esac
done

mkdir -p "$OUT_DIR"

# Scenario list and the op each one measures. "updown" times `up -d` and
# `down -v`; "scale" adds --scale app=5; "reup" times a warm second `up -d`.
SCENARIOS=(single multi-healthcheck scale network-ipam volume-heavy warm-restart many-services)
declare -A OP=(
	[single]=updown [multi-healthcheck]=updown [scale]=scale
	[network-ipam]=updown [volume-heavy]=updown [warm-restart]=reup
	[many-services]=updown
)
if [ "$SMOKE" -eq 1 ]; then SCENARIOS=(single); fi

# Tools available on this host. docker-compose only counts when dockerd is up.
TOOLS=(podup podman-compose)
if docker info >/dev/null 2>&1 && command -v docker-compose >/dev/null 2>&1; then
	TOOLS+=(docker-compose)
	echo "note: Docker Engine present — including docker-compose as a labelled cross-engine run."
else
	echo "note: no Docker Engine on this host — docker-compose (cross-engine) is NOT measured."
fi

run() { # tool, compose-file, project, op-args...
	local tool="$1" file="$2" proj="$3"; shift 3
	local pre=(); [ -n "$CORES" ] && pre=(taskset -c "$CORES")
	case "$tool" in
		podup)          "${pre[@]}" "$PODUP_BIN" -f "$file" -p "$proj" "$@" ;;
		podman-compose) "${pre[@]}" podman-compose -f "$file" -p "$proj" "$@" ;;
		docker-compose) "${pre[@]}" docker-compose -f "$file" -p "$proj" "$@" ;;
	esac
}

timed() { # echoes elapsed seconds for the command, suppressing its output
	local s e
	s=$(date +%s.%N)
	"$@" >/dev/null 2>&1
	local rc=$?
	e=$(date +%s.%N)
	LC_ALL=C awk -v a="$s" -v b="$e" -v rc="$rc" 'BEGIN{ printf "%.6f %d", b-a, rc }'
}

teardown() { run "$1" "$2" "$3" down -v >/dev/null 2>&1; }

echo "tool,scenario,op,iter,phase,seconds,rc" > "$RAW"

for tool in "${TOOLS[@]}"; do
	for scen in "${SCENARIOS[@]}"; do
		file="$SCEN_DIR/$scen/compose.yaml"
		op="${OP[$scen]}"
		proj="bench_${tool//-/_}_${scen//-/_}"
		echo ">>> $tool / $scen (op=$op)"
		teardown "$tool" "$file" "$proj"   # ensure clean slate
		for ((i=0; i<ITERS; i++)); do
			phase="measured"; [ "$i" -lt "$WARMUP" ] && phase="warmup"
			case "$op" in
				updown)
					read -r up_s up_rc <<<"$(timed run "$tool" "$file" "$proj" up -d)"
					read -r dn_s dn_rc <<<"$(timed run "$tool" "$file" "$proj" down -v)"
					echo "$tool,$scen,up,$i,$phase,$up_s,$up_rc" >> "$RAW"
					echo "$tool,$scen,down,$i,$phase,$dn_s,$dn_rc" >> "$RAW"
					;;
				scale)
					read -r up_s up_rc <<<"$(timed run "$tool" "$file" "$proj" up -d --scale app=5)"
					read -r dn_s dn_rc <<<"$(timed run "$tool" "$file" "$proj" down -v)"
					echo "$tool,$scen,up,$i,$phase,$up_s,$up_rc" >> "$RAW"
					echo "$tool,$scen,down,$i,$phase,$dn_s,$dn_rc" >> "$RAW"
					;;
				reup)
					run "$tool" "$file" "$proj" up -d >/dev/null 2>&1            # cold, untimed
					read -r re_s re_rc <<<"$(timed run "$tool" "$file" "$proj" up -d)"   # warm
					echo "$tool,$scen,reup,$i,$phase,$re_s,$re_rc" >> "$RAW"
					teardown "$tool" "$file" "$proj"                              # cleanup, untimed
					;;
			esac
		done
		teardown "$tool" "$file" "$proj"
	done
done

echo "raw rows: $(( $(wc -l < "$RAW") - 1 )) -> $RAW"
