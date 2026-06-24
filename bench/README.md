# Compose-tool benchmark

A reproducible, **fair** wall-clock comparison of compose tools. The point is not
to win — it is to publish honest, equitable numbers across identical scenarios.
A podup loss is published exactly like a podup win.

## What is compared

- **podup** and **podman-compose** both drive **Podman**, so comparing them is a
  pure *tool* comparison — same engine, only the orchestrator differs. This is the
  apples-to-apples result.
- **docker-compose** drives **dockerd**, a different daemon. Any number that
  includes it is an end-to-end *stack* comparison, not a pure-tool one, and is
  labelled as such. It is only measured when a Docker Engine is available on the
  benchmark host; otherwise it is left blank, never estimated.

## Fairness rules (non-negotiable)

- **Identical inputs.** The same compose file per scenario for every tool; images
  are **pinned by digest and pre-pulled**, so image download is never timed.
- **Statistics, not single runs.** N iterations per cell, warm-up discarded,
  reported as **median + p95 + stdev**. A single number is never published.
- **Controlled environment.** The real run happens on a dedicated/self-hosted
  runner or the maintainer's machine, with the CPU governor pinned and the tool
  process taskset-pinned to reduce variance. **Shared CI runners are too noisy for
  published numbers** — CI only runs a smoke check (`--smoke`) that proves the
  harness works, never the numbers in the README.
- **No cherry-picking.** Every scenario is published, whoever wins.

## Scenarios

| scenario | what it exercises |
|---|---|
| `single` | one container — minimal lifecycle cost |
| `multi-healthcheck` | `depends_on: service_healthy` gate on `up` |
| `scale` | `--scale app=5` replica fan-out |
| `network-ipam` | custom bridge network with explicit IPAM |
| `volume-heavy` | several named volumes created/removed |
| `warm-restart` | a second `up` on an already-running project |
| `many-services` | a 12-service compose file |
| `running-ops` | `ps`, `logs`, `exec`, `restart` on a running stack |
| `build` | `build --no-cache` from a Dockerfile (base pinned by digest) |

The lifecycle scenarios time `up -d` and `down -v` (`warm-restart` times the warm
second `up`); `running-ops` brings a stack up untimed, then times each of `ps`,
`logs`, `exec -T`, `restart`; `build` times a `--no-cache` image build. `init:
true` is set on idle `sleep` services so teardown measures the tool, not a
container ignoring `SIGTERM` as PID 1.

## Metrics

Every timed run is wrapped in `/usr/bin/time -v`, so each row records **wall-clock,
peak resident memory (max RSS) and CPU time** of the tool process. The memory and
CPU figures are the **client-side** cost — the tool process and the processes it
directly spawns and waits on. podup is a thin client to the long-running Podman
service, so engine-side work is not charged to it; podman-compose shells out to
`podman` per call and is charged for the work it waits on. The columns therefore
answer "what does invoking the tool cost on my machine", not "how much work does
the engine do".

Wall-clock comes from `/usr/bin/time`'s `Elapsed` line (0.01 s resolution), so
sub-100 ms operations are quantized — equally for both tools, so it limits
resolution symmetrically rather than biasing the comparison.

## Running it

```sh
# build the release binary first; point the harness at it
PODUP_BIN=target/release/podup bench/run.sh --iters 12 --warmup 2 --cores 2-9
python3 bench/aggregate.py
# -> bench/results/report.md and bench/results/summary.json
```

`--smoke` runs a single scenario once (used by CI to check the harness runs).

## Output

`run.sh` writes one raw row per timed run to `results/raw.csv`; `aggregate.py`
discards warm-up and failed runs and computes the statistics into
`results/report.md` + `results/summary.json`. Raw, host-specific results are not
committed; the published numbers live in the repository `README.md`, with the
methodology and host details alongside them.

The harness is reviewed by `podup-benchmark-fairness-auditor` (the harness is
equitable) and `podup-benchmark-results-reviewer` (the published claims are
supported and honest).
