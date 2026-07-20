#!/usr/bin/env bash
# Fair compose-tool benchmark runner.
#
# Drives each tool through the same scenario suite, the same number of times,
# on the same machine, against digest-pinned, pre-pulled images. Each timed run
# is wrapped in /usr/bin/time -v, so every row records wall-clock, peak resident
# memory and CPU time of the orchestrator process; the statistics (median / p95 /
# stdev) are computed by aggregate.py, never here.
#
# Fairness is the whole point: identical compose inputs, identical lifecycle,
# warm-up iterations discarded, every scenario reported, the same op flags for
# every tool. Same-engine tools (podup, podman-compose) drive Podman and are a
# pure tool comparison; docker-compose drives dockerd and is only run when a
# Docker daemon is present, always labelled as an end-to-end stack comparison.
#
# Note on the memory/CPU columns: they are the resource use of the tool process
# and the processes it directly spawns and waits on (getrusage). podup is a thin
# client to the long-running Podman service, so engine-side work is not charged
# to it; podman-compose shells out to the `podman` binary per call, whose work it
# waits on and is charged for. The columns therefore measure client-side cost per
# command — what running the tool costs on your machine — not engine work.
#
# Usage: bench/run.sh [--iters N] [--warmup W] [--cores CPUSET] [--smoke]
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCEN_DIR="$HERE/scenarios"
OUT_DIR="$HERE/results"
RAW="$OUT_DIR/raw.csv"
TIME_BIN="/usr/bin/time"

ITERS=12
WARMUP=2
CORES=""
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

# Scenario list and the op-group each one measures.
#   updown  : time `up -d` and `down -v`
#   scale   : like updown but `up -d --scale app=5`
#   reup    : time a warm second `up -d`
#   running : bring up untimed, then time `ps`, `logs`, `exec -T`, `restart`
#   build   : time `build --no-cache`
SCENARIOS=(single multi-healthcheck deep-chain wide-level scale network-ipam volume-heavy warm-restart many-services running-ops build)
declare -A OP=(
	[single]=updown [multi-healthcheck]=updown [scale]=scale
	[network-ipam]=updown [volume-heavy]=updown [warm-restart]=reup
	[many-services]=updown [running-ops]=running [build]=build
)
if [ "$SMOKE" -eq 1 ]; then SCENARIOS=(single running-ops); fi

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

# Builds the real external command for a tool (so /usr/bin/time can exec it —
# it cannot wrap a shell function) and echoes "wall_s max_rss_kb cpu_s rc".
timed() { # tool, compose-file, project, op-args...
	local tool="$1" file="$2" proj="$3"; shift 3
	local cmd=(); [ -n "$CORES" ] && cmd=(taskset -c "$CORES")
	case "$tool" in
		podup)          cmd+=("$PODUP_BIN" -f "$file" -p "$proj" "$@") ;;
		podman-compose) cmd+=(podman-compose -f "$file" -p "$proj" "$@") ;;
		docker-compose) cmd+=(docker-compose -f "$file" -p "$proj" "$@") ;;
	esac
	local tf; tf="$(mktemp)"
	LC_ALL=C "$TIME_BIN" -v "${cmd[@]}" >/dev/null 2>"$tf"
	local rc=$?
	local wall rss cu cs
	wall="$(grep -m1 'Elapsed' "$tf" | grep -oE '[0-9:.]+$')"
	rss="$(grep -m1 'Maximum resident' "$tf" | grep -oE '[0-9]+$')"
	cu="$(grep -m1 'User time' "$tf" | grep -oE '[0-9.]+$')"
	cs="$(grep -m1 'System time' "$tf" | grep -oE '[0-9.]+$')"
	rm -f "$tf"
	LC_ALL=C awk -v w="$wall" -v r="${rss:-0}" -v u="${cu:-0}" -v s="${cs:-0}" -v rc="$rc" '
		BEGIN{
			n=split(w,p,":"); sec=(n==3)?p[1]*3600+p[2]*60+p[3]:(n==2)?p[1]*60+p[2]:p[1];
			printf "%.6f %d %.3f %d", sec, r, u+s, rc
		}'
}

teardown() { run "$1" "$2" "$3" down -v >/dev/null 2>&1; }

# Pre-pull the digest-pinned bases so image download is never on the timed path.
echo ">>> pre-pulling pinned images"
grep -rhoE 'docker\.io/[^ "]+@sha256:[a-f0-9]+' "$SCEN_DIR" | sort -u | while read -r img; do
	podman pull -q "$img" >/dev/null 2>&1 || echo "  warning: could not pre-pull $img" >&2
done

echo "tool,scenario,op,iter,phase,seconds,max_rss_kb,cpu_s,rc" > "$RAW"

for tool in "${TOOLS[@]}"; do
	for scen in "${SCENARIOS[@]}"; do
		file="$SCEN_DIR/$scen/compose.yaml"
		op="${OP[$scen]}"
		proj="bench_${tool//-/_}_${scen//-/_}"
		echo ">>> $tool / $scen (op=$op)"
		teardown "$tool" "$file" "$proj"
		for ((i=0; i<ITERS; i++)); do
			phase="measured"; [ "$i" -lt "$WARMUP" ] && phase="warmup"
			row() { echo "$tool,$scen,$1,$i,$phase,${2// /,}" >> "$RAW"; }
			case "$op" in
				updown)
					row up   "$(timed "$tool" "$file" "$proj" up -d)"
					row down "$(timed "$tool" "$file" "$proj" down -v)"
					;;
				scale)
					row up   "$(timed "$tool" "$file" "$proj" up -d --scale app=5)"
					row down "$(timed "$tool" "$file" "$proj" down -v)"
					;;
				reup)
					run "$tool" "$file" "$proj" up -d >/dev/null 2>&1
					row reup "$(timed "$tool" "$file" "$proj" up -d)"
					teardown "$tool" "$file" "$proj"
					;;
				running)
					run "$tool" "$file" "$proj" up -d >/dev/null 2>&1
					row ps      "$(timed "$tool" "$file" "$proj" ps)"
					row logs    "$(timed "$tool" "$file" "$proj" logs app)"
					row exec    "$(timed "$tool" "$file" "$proj" exec -T app true)"
					row restart "$(timed "$tool" "$file" "$proj" restart app)"
					teardown "$tool" "$file" "$proj"
					;;
				build)
					row build "$(timed "$tool" "$file" "$proj" build --no-cache)"
					podman rmi -f "podup-bench-build:latest" >/dev/null 2>&1
					;;
			esac
		done
		teardown "$tool" "$file" "$proj"
	done
done

echo "raw rows: $(( $(wc -l < "$RAW") - 1 )) -> $RAW"
