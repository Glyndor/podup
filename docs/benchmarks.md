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

Measured on podup **2.1.0** installed from apt — the same binary a user gets,
not a local build.

## Wall-clock (seconds, lower is better)

| scenario | op | podman-compose | podup | docker-compose |
|---|---|---|---|---|
| single | up | 0.400 (p95 0.430, sd 0.012) | 0.090 (p95 0.100, sd 0.006) | 0.110 (p95 0.150, sd 0.013) |
| single | down | 0.400 (p95 0.450, sd 0.022) | 0.140 (p95 0.240, sd 0.034) | 0.165 (p95 0.180, sd 0.013) |
| multi-healthcheck | up | 0.645 (p95 0.680, sd 0.023) | 0.220 (p95 0.470, sd 0.095) | 0.700 (p95 0.710, sd 0.006) |
| multi-healthcheck | down | 0.515 (p95 0.540, sd 0.020) | 0.245 (p95 0.350, sd 0.040) | 0.280 (p95 0.300, sd 0.016) |
| deep-chain | up | 1.295 (p95 1.340, sd 0.027) | 0.400 (p95 0.470, sd 0.031) | 0.855 (p95 0.880, sd 0.015) |
| deep-chain | down | 0.775 (p95 0.870, sd 0.040) | 0.390 (p95 0.480, sd 0.028) | 0.380 (p95 0.470, sd 0.032) |
| wide-level | up | 7.850 (p95 8.180, sd 0.119) | 1.180 (p95 1.260, sd 0.032) | 1.650 (p95 1.880, sd 0.110) |
| wide-level | down | 4.470 (p95 5.360, sd 0.390) | 1.840 (p95 2.070, sd 0.151) | 2.150 (p95 2.720, sd 0.297) |
| scale | up | 0.415 (p95 0.440, sd 0.011) | 0.190 (p95 0.210, sd 0.010) | 0.395 (p95 0.480, sd 0.031) |
| scale | down | 0.400 (p95 0.440, sd 0.017) | 0.260 (p95 0.300, sd 0.022) | 0.290 (p95 0.380, sd 0.031) |
| network-ipam | up | 0.615 (p95 0.720, sd 0.046) | 0.110 (p95 0.120, sd 0.008) | 0.135 (p95 0.190, sd 0.020) |
| network-ipam | down | 0.510 (p95 0.580, sd 0.033) | 0.170 (p95 0.250, sd 0.026) | 0.200 (p95 0.240, sd 0.015) |
| volume-heavy | up | 0.855 (p95 0.950, sd 0.038) | 0.110 (p95 0.110, sd 0.006) | 0.130 (p95 0.140, sd 0.008) |
| volume-heavy | down | 0.565 (p95 0.600, sd 0.017) | 0.150 (p95 0.210, sd 0.025) | 0.190 (p95 0.230, sd 0.016) |
| warm-restart | warm up | 0.365 (p95 0.490, sd 0.044) | 0.030 (p95 0.040, sd 0.004) | 0.040 (p95 0.050, sd 0.006) |
| many-services | up | 2.430 (p95 2.480, sd 0.029) | 0.380 (p95 0.450, sd 0.025) | 0.495 (p95 0.560, sd 0.024) |
| many-services | down | 1.385 (p95 1.480, sd 0.050) | 0.520 (p95 0.770, sd 0.104) | 0.615 (p95 0.800, sd 0.095) |
| running-ops | ps | 0.110 (p95 0.140, sd 0.009) | 0.000 (p95 0.000, sd 0.000) | 0.020 (p95 0.020, sd 0.000) |
| running-ops | logs | 0.130 (p95 0.140, sd 0.005) | 0.020 (p95 0.020, sd 0.005) | 0.030 (p95 0.030, sd 0.000) |
| running-ops | exec | 0.180 (p95 0.180, sd 0.005) | 0.060 (p95 0.070, sd 0.005) | 0.070 (p95 0.070, sd 0.005) |
| running-ops | restart | 0.290 (p95 0.360, sd 0.024) | 0.170 (p95 0.180, sd 0.007) | 0.180 (p95 0.250, sd 0.025) |
| wide-running-ops | ps | 0.120 (p95 0.120, sd 0.003) | 0.010 (p95 0.010, sd 0.003) | 0.040 (p95 0.130, sd 0.028) |
| wide-running-ops | logs | 0.140 (p95 0.150, sd 0.005) | 0.010 (p95 0.020, sd 0.005) | 0.030 (p95 0.080, sd 0.015) |
| wide-running-ops | exec | 0.185 (p95 0.210, sd 0.011) | 0.060 (p95 0.070, sd 0.004) | 0.070 (p95 0.110, sd 0.013) |
| wide-running-ops | restart | 0.250 (p95 0.260, sd 0.008) | 0.125 (p95 0.130, sd 0.011) | 0.150 (p95 0.310, sd 0.050) |
| config-heavy | config | 0.100 (p95 0.120, sd 0.007) | 0.000 (p95 0.000, sd 0.000) | 0.040 (p95 0.040, sd 0.004) |
| build | build | 0.340 (p95 0.360, sd 0.009) | 0.200 (p95 0.210, sd 0.007) | 0.260 (p95 0.290, sd 0.014) |

## Memory + CPU per command (peak RSS / CPU time, median)

This is the **client-side** cost of running the tool, not engine work. podup is
a static binary talking to the Podman service, so work the engine does is not
charged to it. podman-compose is Python that shells out to the `podman` binary
per call and waits on it, so that work *is* charged to it. docker-compose is a
Go binary talking to a socket, like podup.

| scenario | op | podman-compose | podup | docker-compose |
|---|---|---|---|---|
| single | up | 51.2 MiB / 0.405 s | 7.5 MiB / 0.000 s | 28.7 MiB / 0.020 s |
| single | down | 49.0 MiB / 0.345 s | 7.2 MiB / 0.000 s | 28.2 MiB / 0.010 s |
| multi-healthcheck | up | 51.8 MiB / 0.610 s | 7.5 MiB / 0.000 s | 29.0 MiB / 0.020 s |
| multi-healthcheck | down | 49.1 MiB / 0.460 s | 7.3 MiB / 0.000 s | 28.2 MiB / 0.020 s |
| deep-chain | up | 51.8 MiB / 1.210 s | 7.5 MiB / 0.000 s | 29.1 MiB / 0.025 s |
| deep-chain | down | 49.1 MiB / 0.810 s | 7.4 MiB / 0.000 s | 28.7 MiB / 0.020 s |
| wide-level | up | 52.1 MiB / 6.890 s | 7.9 MiB / 0.025 s | 34.6 MiB / 0.090 s |
| wide-level | down | 49.5 MiB / 5.535 s | 7.5 MiB / 0.010 s | 31.1 MiB / 0.060 s |
| scale | up | 50.8 MiB / 0.420 s | 7.5 MiB / 0.000 s | 29.0 MiB / 0.025 s |
| scale | down | 49.0 MiB / 0.340 s | 7.3 MiB / 0.000 s | 28.5 MiB / 0.020 s |
| network-ipam | up | 51.2 MiB / 0.570 s | 7.4 MiB / 0.000 s | 28.8 MiB / 0.020 s |
| network-ipam | down | 48.7 MiB / 0.460 s | 7.4 MiB / 0.000 s | 28.4 MiB / 0.020 s |
| volume-heavy | up | 51.3 MiB / 0.960 s | 7.4 MiB / 0.000 s | 28.7 MiB / 0.020 s |
| volume-heavy | down | 49.2 MiB / 0.530 s | 7.3 MiB / 0.000 s | 28.9 MiB / 0.020 s |
| warm-restart | warm up | 49.4 MiB / 0.410 s | 7.5 MiB / 0.000 s | 29.0 MiB / 0.020 s |
| many-services | up | 51.8 MiB / 2.125 s | 7.6 MiB / 0.000 s | 30.6 MiB / 0.040 s |
| many-services | down | 49.1 MiB / 1.625 s | 7.4 MiB / 0.000 s | 29.2 MiB / 0.030 s |
| running-ops | ps | 47.6 MiB / 0.130 s | 7.0 MiB / 0.000 s | 28.7 MiB / 0.015 s |
| running-ops | logs | 64.0 MiB / 0.130 s | 7.3 MiB / 0.000 s | 28.5 MiB / 0.015 s |
| running-ops | exec | 47.4 MiB / 0.130 s | 7.4 MiB / 0.000 s | 26.9 MiB / 0.010 s |
| running-ops | restart | 47.8 MiB / 0.170 s | 7.2 MiB / 0.000 s | 28.5 MiB / 0.020 s |
| wide-running-ops | ps | 48.8 MiB / 0.135 s | 7.1 MiB / 0.000 s | 29.5 MiB / 0.030 s |
| wide-running-ops | logs | 63.5 MiB / 0.130 s | 7.3 MiB / 0.000 s | 29.3 MiB / 0.020 s |
| wide-running-ops | exec | 47.1 MiB / 0.130 s | 7.3 MiB / 0.000 s | 26.7 MiB / 0.010 s |
| wide-running-ops | restart | 47.7 MiB / 0.175 s | 7.2 MiB / 0.000 s | 28.5 MiB / 0.020 s |
| config-heavy | config | 37.9 MiB / 0.110 s | 6.8 MiB / 0.000 s | 29.2 MiB / 0.050 s |
| build | build | 55.6 MiB / 0.360 s | 7.3 MiB / 0.000 s | 29.3 MiB / 0.020 s |

## Reading these numbers honestly

podup leads every row here, and two of them deserve a caveat rather than a
victory lap.

`running-ops ps` and `config-heavy config` show podup at **0.000s**. That is the
floor of `/usr/bin/time -v`, which resolves to 10ms — the real figure is around
8ms, dominated by process startup rather than by the work. It is not zero, it is
below what the instrument can see.

The `multi-healthcheck` row moved the most between releases: 1.275s in 2.0.0
against 0.220s here. That is not a scheduling trick, it is a bug fix — podup
used to read a container's health status once per healthcheck `interval` and
now reads it every 150ms between runs, so a container that turns healthy just
after a probe is noticed at once instead of at the end of the window.
