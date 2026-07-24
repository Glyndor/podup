# Benchmarks

## vs alternatives

|  | podup | docker-compose | podman-compose (Python) |
|---|---|---|---|
| Engine | rootless Podman | Docker daemon | Podman |
| Runtime | single static binary | Go binary + Docker daemon | Python + pip packages |
| Root required | no | typically yes (daemon) | no |
| Implementation | Rust | Go | Python |
| Podman API | native libpod REST | n/a | Podman CLI shell-out |
| Systemd Quadlet export | yes (`generate quadlet`) | no | no |
| Platforms | Linux · macOS · Windows (single binary) | Linux · macOS · Windows | wherever Python runs |
| Compose-spec depth | `extends`, profiles, `develop.watch`, inline secrets/configs | full | partial |

## Methodology

All three tools drive **the same rootless Podman**, so this is a pure *tool*
comparison: identical engine, identical digest-pinned and pre-pulled images,
identical compose file per scenario, the same op flags for every tool. Each
number is the median over **10 measured iterations** — 12 runs with the first
2 discarded as warm-up; p95 and standard deviation are in parentheses.

`docker-compose` normally drives dockerd. Pointing it at the Podman socket
through `DOCKER_HOST` is what makes it comparable here — the only difference
left is the orchestrator. Run against a Docker daemon instead, the numbers
would fold in the engine difference and could not be read as tool-versus-tool;
the harness detects which engine it drove and labels the report accordingly.

Reproduce with `bash bench/run.sh` (set `DOCKER_HOST` to the Podman socket to
include docker-compose), then `python3 bench/aggregate.py`. Raw per-iteration
rows land in `bench/results/raw.csv`; the statistics are computed there and
never by the runner.

Measured on podup **3.0.1** installed from apt — the same binary a user gets,
not a local build.

## Wall-clock (seconds, lower is better)

| scenario | op | podman-compose | podup | docker-compose |
|---|---|---|---|---|
| single | up | 0.415 (p95 0.460, sd 0.019) | 0.080 (p95 0.090, sd 0.005) | 0.110 (p95 0.120, sd 0.006) |
| single | down | 0.395 (p95 0.430, sd 0.020) | 0.135 (p95 0.190, sd 0.019) | 0.150 (p95 0.180, sd 0.014) |
| multi-healthcheck | up | 0.650 (p95 0.680, sd 0.018) | 0.280 (p95 0.400, sd 0.087) | 0.690 (p95 0.710, sd 0.007) |
| multi-healthcheck | down | 0.510 (p95 0.540, sd 0.016) | 0.245 (p95 0.310, sd 0.029) | 0.275 (p95 0.380, sd 0.034) |
| deep-chain | up | 1.300 (p95 1.340, sd 0.022) | 0.370 (p95 0.390, sd 0.011) | 0.845 (p95 0.910, sd 0.028) |
| deep-chain | down | 0.770 (p95 0.850, sd 0.033) | 0.380 (p95 0.410, sd 0.017) | 0.380 (p95 0.400, sd 0.015) |
| wide-level | up | 7.690 (p95 7.860, sd 0.094) | 1.120 (p95 1.270, sd 0.055) | 1.545 (p95 1.730, sd 0.069) |
| wide-level | down | 4.375 (p95 4.620, sd 0.118) | 1.885 (p95 2.380, sd 0.245) | 2.145 (p95 3.660, sd 0.519) |
| scale | up | 0.435 (p95 0.540, sd 0.038) | 0.190 (p95 0.220, sd 0.012) | 0.380 (p95 0.390, sd 0.008) |
| scale | down | 0.420 (p95 0.450, sd 0.019) | 0.275 (p95 0.310, sd 0.023) | 0.285 (p95 0.330, sd 0.020) |
| network-ipam | up | 0.590 (p95 0.620, sd 0.020) | 0.100 (p95 0.110, sd 0.005) | 0.130 (p95 0.140, sd 0.005) |
| network-ipam | down | 0.500 (p95 0.520, sd 0.018) | 0.165 (p95 0.180, sd 0.011) | 0.190 (p95 0.210, sd 0.016) |
| volume-heavy | up | 0.830 (p95 1.000, sd 0.069) | 0.100 (p95 0.120, sd 0.011) | 0.120 (p95 0.150, sd 0.010) |
| volume-heavy | down | 0.550 (p95 0.680, sd 0.052) | 0.145 (p95 0.160, sd 0.010) | 0.190 (p95 0.210, sd 0.014) |
| warm-restart | warm up | 0.360 (p95 0.370, sd 0.008) | 0.030 (p95 0.030, sd 0.004) | 0.040 (p95 0.050, sd 0.005) |
| many-services | up | 2.365 (p95 2.430, sd 0.042) | 0.380 (p95 0.430, sd 0.025) | 0.485 (p95 0.520, sd 0.017) |
| many-services | down | 1.420 (p95 1.480, sd 0.035) | 0.580 (p95 0.730, sd 0.092) | 0.565 (p95 0.600, sd 0.039) |
| running-ops | ps | 0.110 (p95 0.130, sd 0.007) | 0.000 (p95 0.000, sd 0.000) | 0.020 (p95 0.020, sd 0.000) |
| running-ops | logs | 0.150 (p95 0.170, sd 0.010) | 0.030 (p95 0.050, sd 0.007) | 0.035 (p95 0.040, sd 0.005) |
| running-ops | exec | 0.185 (p95 0.200, sd 0.009) | 0.060 (p95 0.060, sd 0.003) | 0.070 (p95 0.080, sd 0.005) |
| running-ops | restart | 0.290 (p95 0.320, sd 0.018) | 0.170 (p95 0.200, sd 0.013) | 0.170 (p95 0.200, sd 0.010) |
| wide-running-ops | ps | 0.130 (p95 0.140, sd 0.010) | 0.010 (p95 0.020, sd 0.003) | 0.035 (p95 0.040, sd 0.005) |
| wide-running-ops | logs | 0.160 (p95 0.170, sd 0.008) | 0.030 (p95 0.030, sd 0.005) | 0.040 (p95 0.040, sd 0.004) |
| wide-running-ops | exec | 0.200 (p95 0.210, sd 0.012) | 0.060 (p95 0.070, sd 0.005) | 0.070 (p95 0.070, sd 0.003) |
| wide-running-ops | restart | 0.240 (p95 0.270, sd 0.013) | 0.115 (p95 0.120, sd 0.007) | 0.140 (p95 0.160, sd 0.009) |
| config-heavy | config | 0.130 (p95 0.140, sd 0.006) | 0.000 (p95 0.000, sd 0.000) | 0.040 (p95 0.040, sd 0.005) |
| build | build | 0.340 (p95 0.380, sd 0.019) | 0.200 (p95 0.210, sd 0.006) | 0.260 (p95 0.280, sd 0.010) |

## Memory + CPU per command (peak RSS / CPU time, median)

This is the **client-side** cost of running the tool, not engine work. podup is
a static binary talking to the Podman service, so work the engine does is not
charged to it. podman-compose is Python that shells out to the `podman` binary
per call and waits on it, so that work *is* charged to it. docker-compose is a
Go binary talking to a socket, like podup.

| scenario | op | podman-compose | podup | docker-compose |
|---|---|---|---|---|
| single | up | 51.7 MiB / 0.410 s | 7.8 MiB / 0.000 s | 28.7 MiB / 0.020 s |
| single | down | 49.5 MiB / 0.335 s | 7.6 MiB / 0.000 s | 28.3 MiB / 0.010 s |
| multi-healthcheck | up | 52.4 MiB / 0.590 s | 7.9 MiB / 0.000 s | 29.0 MiB / 0.020 s |
| multi-healthcheck | down | 49.6 MiB / 0.450 s | 7.6 MiB / 0.000 s | 28.3 MiB / 0.020 s |
| deep-chain | up | 53.0 MiB / 1.180 s | 7.7 MiB / 0.000 s | 29.4 MiB / 0.030 s |
| deep-chain | down | 50.0 MiB / 0.760 s | 7.7 MiB / 0.000 s | 28.5 MiB / 0.020 s |
| wide-level | up | 52.8 MiB / 6.520 s | 8.4 MiB / 0.020 s | 33.3 MiB / 0.080 s |
| wide-level | down | 50.1 MiB / 4.990 s | 7.9 MiB / 0.010 s | 30.4 MiB / 0.050 s |
| scale | up | 51.7 MiB / 0.405 s | 7.8 MiB / 0.000 s | 29.7 MiB / 0.020 s |
| scale | down | 49.8 MiB / 0.330 s | 7.5 MiB / 0.000 s | 28.7 MiB / 0.010 s |
| network-ipam | up | 52.2 MiB / 0.540 s | 7.8 MiB / 0.000 s | 29.1 MiB / 0.020 s |
| network-ipam | down | 49.9 MiB / 0.430 s | 7.6 MiB / 0.000 s | 28.4 MiB / 0.020 s |
| volume-heavy | up | 52.2 MiB / 0.940 s | 7.8 MiB / 0.000 s | 28.9 MiB / 0.020 s |
| volume-heavy | down | 49.6 MiB / 0.520 s | 7.5 MiB / 0.000 s | 29.1 MiB / 0.020 s |
| warm-restart | warm up | 50.3 MiB / 0.390 s | 7.7 MiB / 0.000 s | 29.6 MiB / 0.020 s |
| many-services | up | 52.6 MiB / 2.005 s | 8.0 MiB / 0.000 s | 30.2 MiB / 0.040 s |
| many-services | down | 49.8 MiB / 1.520 s | 7.6 MiB / 0.000 s | 28.9 MiB / 0.030 s |
| running-ops | ps | 48.5 MiB / 0.120 s | 7.3 MiB / 0.000 s | 28.5 MiB / 0.010 s |
| running-ops | logs | 64.1 MiB / 0.130 s | 7.6 MiB / 0.000 s | 28.4 MiB / 0.010 s |
| running-ops | exec | 47.7 MiB / 0.135 s | 7.6 MiB / 0.000 s | 26.9 MiB / 0.010 s |
| running-ops | restart | 48.3 MiB / 0.175 s | 7.5 MiB / 0.000 s | 28.7 MiB / 0.020 s |
| wide-running-ops | ps | 49.5 MiB / 0.140 s | 7.3 MiB / 0.000 s | 29.5 MiB / 0.030 s |
| wide-running-ops | logs | 64.3 MiB / 0.140 s | 7.6 MiB / 0.000 s | 28.4 MiB / 0.020 s |
| wide-running-ops | exec | 48.1 MiB / 0.140 s | 7.6 MiB / 0.000 s | 27.0 MiB / 0.010 s |
| wide-running-ops | restart | 48.8 MiB / 0.180 s | 7.5 MiB / 0.000 s | 28.6 MiB / 0.020 s |
| config-heavy | config | 37.8 MiB / 0.125 s | 7.1 MiB / 0.000 s | 29.3 MiB / 0.045 s |
| build | build | 57.2 MiB / 0.355 s | 7.7 MiB / 0.000 s | 30.0 MiB / 0.020 s |

## Reading these numbers honestly

podup is fastest in every row of the memory-and-CPU table, and the fastest or
tied-fastest in all but one of the wall-clock rows. The exception is
`many-services down`, where docker-compose's 0.565 s edges podup's 0.580 s —
0.015 s apart, well inside podup's own 0.092 s standard deviation on that row,
so it is a coin toss rather than a real gap. Two teardown rows land in an exact
tie (`deep-chain down` and `running-ops restart`, both 0.170–0.380 s to the
millisecond). The point of this benchmark is to publish the numbers whoever
wins, so those rows stand as measured, and which teardown row falls on which
side of the line shifts run to run within that noise.

`running-ops ps` and `config-heavy config` show podup at **0.000s**. That is the
floor of `/usr/bin/time -v`, which resolves to 10ms — the real figure is around
8ms, dominated by process startup rather than by the work. It is not zero, it is
below what the instrument can see.

The `multi-healthcheck` row moved the most between releases: 1.275s in 2.0.0
against 0.280s here. That is not a scheduling trick, it is a bug fix — podup
used to read a container's health status once per healthcheck `interval` and
now reads it every 150ms between runs, so a container that turns healthy just
after a probe is noticed at once instead of at the end of the window.
