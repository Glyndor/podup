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

**podup** and **podman-compose** both drive the same Podman, so this is a pure
*tool* comparison — identical engine, identical digest-pinned and pre-pulled
images, identical compose file per scenario, the same op flags for both. Each
number is the median over 10 measured iterations (2 warm-up runs discarded).

## Wall-clock (seconds, lower is better)

| scenario | op | podup | podman-compose |
|---|---|---|---|
| single | up | **0.100** (p95 0.100, sd 0.005) | 0.660 (p95 0.700, sd 0.014) |
| single | down | **0.140** (p95 0.150, sd 0.008) | 0.585 (p95 0.610, sd 0.014) |
| multi-healthcheck | up | **0.260** (p95 1.310, sd 0.520) | 1.035 (p95 1.390, sd 0.112) |
| multi-healthcheck | down | **0.285** (p95 0.310, sd 0.027) | 0.750 (p95 1.020, sd 0.083) |
| scale (×5) | up | **0.405** (p95 0.420, sd 0.012) | 0.690 (p95 0.710, sd 0.012) |
| scale (×5) | down | **0.430** (p95 0.450, sd 0.015) | 0.605 (p95 0.620, sd 0.018) |
| network + IPAM | up | **0.120** (p95 0.130, sd 0.010) | 0.960 (p95 1.010, sd 0.026) |
| network + IPAM | down | **0.210** (p95 0.230, sd 0.008) | 0.745 (p95 0.770, sd 0.022) |
| volume-heavy | up | **0.110** (p95 0.120, sd 0.007) | 1.500 (p95 1.550, sd 0.026) |
| volume-heavy | down | **0.150** (p95 0.170, sd 0.008) | 0.870 (p95 0.900, sd 0.021) |
| warm restart | warm up | **0.020** (p95 0.040, sd 0.006) | 0.600 (p95 0.620, sd 0.014) |
| many-services (12) | up | **0.845** (p95 0.910, sd 0.049) | 3.835 (p95 3.890, sd 0.108) |
| many-services (12) | down | **1.670** (p95 1.830, sd 0.102) | 1.925 (p95 2.010, sd 0.046) |
| running stack | ps | **0.015** (p95 0.020, sd 0.005) | 0.130 (p95 0.150, sd 0.007) |
| running stack | logs | **0.015** (p95 0.050, sd 0.012) | 0.160 (p95 0.170, sd 0.006) |
| running stack | exec | **0.070** (p95 0.140, sd 0.023) | 0.200 (p95 0.210, sd 0.005) |
| running stack | restart | **0.220** (p95 0.260, sd 0.021) | 0.350 (p95 0.370, sd 0.017) |
| build (`--no-cache`) | build | **0.285** (p95 0.310, sd 0.013) | 0.420 (p95 0.440, sd 0.008) |

## Memory + CPU per command (peak RSS / CPU time, median)

This is the **client-side** cost of running the tool. podup is a static binary
that hands the work to the long-running Podman service, so its own CPU is near
zero and its memory is flat; podman-compose is Python that shells out to `podman`
on every call and is charged for the work it waits on. (The engine does the
container work either way — this measures what invoking the *tool* costs.)

| scenario | op | podup | podman-compose |
|---|---|---|---|
| single | up | **7.2 MiB / 0.00 s** | 69.2 MiB / 0.66 s |
| volume-heavy | up | **7.2 MiB / 0.00 s** | 68.8 MiB / 1.67 s |
| many-services (12) | up | **7.4 MiB / 0.01 s** | 70.2 MiB / 3.29 s |
| running stack | ps | **6.9 MiB / 0.00 s** | 59.1 MiB / 0.15 s |
| running stack | logs | **6.9 MiB / 0.00 s** | 72.8 MiB / 0.15 s |
| running stack | exec | **7.0 MiB / 0.00 s** | 59.4 MiB / 0.16 s |
| build (`--no-cache`) | build | **7.0 MiB / 0.00 s** | 88.9 MiB / 0.48 s |

Across **every** op podup stays around **7 MiB** of peak memory and near-zero
client CPU; podman-compose ranges **59–89 MiB** and **0.15–3.3 s** of CPU.

Host: AMD Ryzen 7 5700X (16 threads), Linux 6.17 x86_64, CPU governor
`performance`, Podman 5.4.2, podman-compose 1.3.0; the tool process pinned with
`taskset`. Measured 2026-06-23. On `multi-healthcheck` the high p95/stdev on
podup's `up` is the `service_healthy` gate — it waits on the dependency's
healthcheck interval, so the tail varies; the median still leads.

> **docker-compose is not in these tables.** It drives `dockerd`, a different
> daemon, so including it would be an end-to-end *stack* comparison, not a
> pure-tool one — and this host had no Docker Engine, so it is left out rather
> than estimated.

podup is faster and lighter on every operation measured here, widest on the
volume- and service-heavy stacks. The harness, scenarios, and full methodology
live in [`bench/`](../bench/); reproduce with `bench/run.sh`. Every scenario is
published, whoever wins.
