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

Two comparisons, measured in one run on one machine:

- **Same engine.** podup, podman-compose and docker-compose all drive **the same
  rootless Podman**; docker-compose is pointed at the Podman socket through
  `DOCKER_HOST`. The only difference left is the compose tool, so this is the
  tool-against-tool result.
- **Each tool on its own engine.** podup and podman-compose on rootless Podman,
  docker-compose on the Docker daemon (rootful). This is what a user of each
  stack sees; the engines differ, so it cannot be read as tool against tool.

Identical digest-pinned images, pre-pulled into both engines so no download is
timed; the same compose file per scenario; the same op flags for every tool.
Each number is the median over **10 measured iterations**: 12 runs with the first
2 discarded as warm-up; p95 and standard deviation are in parentheses. A row with
a failed iteration is refused rather than published over the survivors, and this
run had none: 1392 timed runs, 0 failed.

Reproduce with `bash bench/run.sh`, then `python3 bench/aggregate.py`; the harness
measures both docker-compose variants when both engines answer, and writes the
host description below into `bench/results/env.txt`. Timing comes from
`bench/timeit` (fork, clock across the command, peak RSS and CPU from `wait4`'s
rusage).

Each row carries **one unit**, picked from the largest value in that row and
applied to every tool in it. `bench/results/raw.csv` and `summary.json` keep every
figure in seconds.

Measured on podup **5.10.5**, the published `podup-linux-x86_64` asset (static
musl, the binary the installers fetch, checked against `SHA256SUMS`):

```
kernel: 7.0.0-34-generic
cpu: AMD Ryzen 7 5700X 8-Core Processor (16 threads), governor performance, tools pinned to cores 2-9
podman: 5.7.0 (Ubuntu 26.04)
podman-compose: 1.6.0
docker-compose: 5.5.1
docker server: 29.8.1
podman store at start: 18 containers, 30 networks, 31 volumes, 226 images, 1 dangling
running VMs: 0
```

**Do not compare these numbers with the 5.7.1 table this page used to carry.**
The Podman engine on this host is two to three times slower than it was then on
the multi-container rows, and that is not podup: the 5.7.1 and 5.10.5 binaries,
run against each other on this host at the same moment, take the same time
(`wide-level up` 2.90 s against 2.99 s, `down` 5.37 s against 5.18 s, medians of
three), and docker-compose on the same socket slowed by a similar factor. Stopping
Docker and holding the rootless network namespace open changed nothing, so the
cause is somewhere in the engine or the host and is not identified. It does not
bear on the comparison below, where every tool ran on the same engine in the same
run.

## Wall-clock, same engine (lower is better)

| scenario | op | podup | podman-compose | docker-compose (Podman) |
|---|---|---|---|---|
| single | up | 100.4 ms (p95 111.9, sd 5.8) | 446.0 ms (p95 475.2, sd 14.0) | 147.2 ms (p95 153.3, sd 5.0) |
| single | down | 127.8 ms (p95 138.1, sd 6.6) | 442.9 ms (p95 470.2, sd 13.7) | 211.1 ms (p95 248.3, sd 14.0) |
| multi-healthcheck | up | 0.364 s (p95 0.398, sd 0.050) | 0.981 s (p95 1.014, sd 0.025) | 0.797 s (p95 0.835, sd 0.014) |
| multi-healthcheck | down | 284.3 ms (p95 313.6, sd 26.6) | 624.4 ms (p95 697.9, sd 50.2) | 390.9 ms (p95 418.4, sd 18.8) |
| deep-chain | up | 0.372 s (p95 0.576, sd 0.064) | 1.763 s (p95 1.819, sd 0.034) | 1.040 s (p95 1.140, sd 0.040) |
| deep-chain | down | 0.417 s (p95 0.440, sd 0.018) | 1.096 s (p95 1.281, sd 0.071) | 0.744 s (p95 0.781, sd 0.027) |
| wide-level | up | 2.831 s (p95 3.071, sd 0.854) | 8.510 s (p95 8.656, sd 0.131) | 4.721 s (p95 5.611, sd 0.293) |
| wide-level | down | 5.155 s (p95 6.275, sd 1.319) | 7.429 s (p95 9.890, sd 0.983) | 5.322 s (p95 6.614, sd 0.532) |
| scale | up | 432.4 ms (p95 877.4, sd 180.1) | 476.7 ms (p95 493.2, sd 9.6) | 567.6 ms (p95 594.0, sd 21.4) |
| scale | down | 0.689 s (p95 1.762, sd 0.329) | 0.420 s (p95 0.427, sd 0.010) | 0.697 s (p95 0.789, sd 0.067) |
| network-ipam | up | 204.5 ms (p95 216.5, sd 6.5) | 679.7 ms (p95 700.5, sd 16.8) | 250.7 ms (p95 256.6, sd 6.3) |
| network-ipam | down | 255.6 ms (p95 276.7, sd 9.4) | 594.1 ms (p95 643.7, sd 19.5) | 274.5 ms (p95 308.2, sd 14.1) |
| volume-heavy | up | 208.0 ms (p95 221.3, sd 8.3) | 948.3 ms (p95 963.2, sd 6.2) | 252.1 ms (p95 272.4, sd 11.5) |
| volume-heavy | down | 264.2 ms (p95 291.4, sd 13.4) | 665.6 ms (p95 680.1, sd 8.6) | 345.3 ms (p95 374.0, sd 18.9) |
| secrets | up | 164.9 ms (p95 196.4, sd 14.4) | 493.0 ms (p95 501.4, sd 7.3) | 147.8 ms (p95 156.5, sd 7.1) |
| secrets | down | 219.2 ms (p95 249.2, sd 14.0) | 476.4 ms (p95 487.9, sd 9.3) | 215.5 ms (p95 251.1, sd 14.7) |
| warm-restart | warm up | 45.2 ms (p95 63.4, sd 9.0) | 262.9 ms (p95 273.0, sd 6.0) | 45.3 ms (p95 49.6, sd 4.0) |
| many-services | up | 0.964 s (p95 1.073, sd 0.095) | 2.560 s (p95 2.645, sd 0.044) | 1.359 s (p95 1.757, sd 0.126) |
| many-services | down | 1.518 s (p95 1.731, sd 0.092) | 2.182 s (p95 2.328, sd 0.084) | 1.601 s (p95 3.236, sd 0.495) |
| running-ops | ps | 5.6 ms (p95 7.3, sd 0.6) | 115.2 ms (p95 117.6, sd 1.2) | 26.1 ms (p95 26.7, sd 0.7) |
| running-ops | logs | 5.5 ms (p95 6.7, sd 0.4) | 159.2 ms (p95 169.7, sd 4.5) | 49.0 ms (p95 52.2, sd 3.1) |
| running-ops | exec | 82.8 ms (p95 88.3, sd 3.6) | 234.5 ms (p95 243.8, sd 4.1) | 96.5 ms (p95 109.1, sd 5.5) |
| running-ops | restart | 231.7 ms (p95 252.0, sd 10.2) | 319.9 ms (p95 354.0, sd 13.9) | 231.3 ms (p95 260.2, sd 17.2) |
| wide-running-ops | ps | 9.7 ms (p95 11.3, sd 0.8) | 182.5 ms (p95 193.1, sd 3.5) | 40.6 ms (p95 42.6, sd 1.4) |
| wide-running-ops | logs | 9.3 ms (p95 9.8, sd 0.7) | 212.1 ms (p95 215.8, sd 3.0) | 50.2 ms (p95 51.8, sd 3.2) |
| wide-running-ops | exec | 90.6 ms (p95 100.2, sd 4.7) | 280.8 ms (p95 285.8, sd 4.0) | 99.0 ms (p95 112.5, sd 6.0) |
| wide-running-ops | restart | 174.7 ms (p95 192.6, sd 8.0) | 334.8 ms (p95 345.9, sd 5.3) | 204.0 ms (p95 212.0, sd 8.8) |
| config-heavy | config | 12.0 ms (p95 17.7, sd 1.7) | 538.3 ms (p95 549.6, sd 4.3) | 33.8 ms (p95 37.8, sd 1.6) |
| build | build | 0.437 s (p95 0.440, sd 0.004) | 0.470 s (p95 0.487, sd 0.030) | 1.700 s (p95 2.549, sd 0.260) |

## Memory + CPU per command, same engine (peak RSS / CPU time, median)

Client-side cost of invoking the tool: the tool process and what it spawns and
waits on. podup is a static binary talking to the Podman service; podman-compose
is Python shelling out to `podman` per call and is charged for that work.

| scenario | op | podup | podman-compose | docker-compose (Podman) |
|---|---|---|---|---|
| single | up | 7.2 MiB / 5.9 ms | 51.5 MiB / 471.6 ms | 29.7 MiB / 34.1 ms |
| single | down | 7.7 MiB / 7.4 ms | 50.0 MiB / 351.0 ms | 29.4 MiB / 31.9 ms |
| multi-healthcheck | up | 7.2 MiB / 7.7 ms | 52.0 MiB / 727.4 ms | 29.8 MiB / 38.1 ms |
| multi-healthcheck | down | 7.6 MiB / 8.0 ms | 50.3 MiB / 478.1 ms | 29.3 MiB / 33.0 ms |
| deep-chain | up | 7.3 MiB / 0.009 s | 52.5 MiB / 1.334 s | 30.1 MiB / 0.042 s |
| deep-chain | down | 7.7 MiB / 9.9 ms | 50.5 MiB / 846.6 ms | 29.2 MiB / 36.2 ms |
| wide-level | up | 8.0 MiB / 0.031 s | 53.1 MiB / 7.003 s | 34.6 MiB / 0.093 s |
| wide-level | down | 8.0 MiB / 0.026 s | 51.4 MiB / 5.451 s | 31.4 MiB / 0.069 s |
| scale | up | 7.3 MiB / 8.0 ms | 51.6 MiB / 498.0 ms | 30.1 MiB / 39.9 ms |
| scale | down | 7.8 MiB / 8.9 ms | 50.0 MiB / 347.2 ms | 29.3 MiB / 35.3 ms |
| network-ipam | up | 7.2 MiB / 6.7 ms | 52.1 MiB / 625.9 ms | 29.9 MiB / 36.7 ms |
| network-ipam | down | 7.7 MiB / 7.9 ms | 50.1 MiB / 476.0 ms | 29.3 MiB / 33.1 ms |
| volume-heavy | up | 7.2 MiB / 0.007 s | 52.2 MiB / 1.054 s | 30.4 MiB / 0.039 s |
| volume-heavy | down | 7.8 MiB / 8.5 ms | 49.6 MiB / 571.1 ms | 29.6 MiB / 36.8 ms |
| secrets | up | 7.2 MiB / 7.7 ms | 51.3 MiB / 474.0 ms | 29.8 MiB / 35.6 ms |
| secrets | down | 7.8 MiB / 8.3 ms | 49.8 MiB / 368.4 ms | 29.3 MiB / 32.6 ms |
| warm-restart | warm up | 7.2 MiB / 6.7 ms | 47.1 MiB / 314.1 ms | 30.1 MiB / 34.4 ms |
| many-services | up | 7.5 MiB / 0.012 s | 53.1 MiB / 2.209 s | 31.5 MiB / 0.053 s |
| many-services | down | 7.9 MiB / 0.012 s | 50.8 MiB / 1.651 s | 29.9 MiB / 0.043 s |
| running-ops | ps | 6.9 MiB / 4.2 ms | 46.5 MiB / 129.0 ms | 29.2 MiB / 29.8 ms |
| running-ops | logs | 6.9 MiB / 4.2 ms | 62.1 MiB / 139.4 ms | 29.4 MiB / 30.7 ms |
| running-ops | exec | 6.9 MiB / 4.8 ms | 47.2 MiB / 142.0 ms | 27.1 MiB / 18.9 ms |
| running-ops | restart | 6.9 MiB / 4.7 ms | 47.9 MiB / 178.7 ms | 29.2 MiB / 30.9 ms |
| wide-running-ops | ps | 6.9 MiB / 5.3 ms | 47.8 MiB / 201.0 ms | 30.4 MiB / 40.8 ms |
| wide-running-ops | logs | 6.9 MiB / 5.2 ms | 62.4 MiB / 204.8 ms | 29.6 MiB / 35.4 ms |
| wide-running-ops | exec | 7.0 MiB / 5.7 ms | 47.3 MiB / 204.8 ms | 27.1 MiB / 18.8 ms |
| wide-running-ops | restart | 6.9 MiB / 5.6 ms | 48.0 MiB / 236.1 ms | 29.6 MiB / 36.2 ms |
| config-heavy | config | 7.4 MiB / 12.7 ms | 34.5 MiB / 542.0 ms | 30.5 MiB / 50.1 ms |
| build | build | 7.5 MiB / 5.4 ms | 61.6 MiB / 381.7 ms | 61.0 MiB / 193.2 ms |

## Wall-clock, each tool on its own engine

podup and podman-compose on rootless Podman, docker-compose on dockerd. The podup
and podman-compose columns are the same measurements as above.

| scenario | op | podup | podman-compose | docker-compose (Docker) |
|---|---|---|---|---|
| single | up | 100.4 ms (p95 111.9, sd 5.8) | 446.0 ms (p95 475.2, sd 14.0) | 270.4 ms (p95 287.6, sd 7.8) |
| single | down | 127.8 ms (p95 138.1, sd 6.6) | 442.9 ms (p95 470.2, sd 13.7) | 296.6 ms (p95 323.4, sd 14.5) |
| multi-healthcheck | up | 0.364 s (p95 0.398, sd 0.050) | 0.981 s (p95 1.014, sd 0.025) | 1.508 s (p95 1.521, sd 0.008) |
| multi-healthcheck | down | 284.3 ms (p95 313.6, sd 26.6) | 624.4 ms (p95 697.9, sd 50.2) | 512.2 ms (p95 546.0, sd 15.7) |
| deep-chain | up | 0.372 s (p95 0.576, sd 0.064) | 1.763 s (p95 1.819, sd 0.034) | 2.381 s (p95 2.575, sd 0.067) |
| deep-chain | down | 0.417 s (p95 0.440, sd 0.018) | 1.096 s (p95 1.281, sd 0.071) | 0.789 s (p95 0.831, sd 0.020) |
| wide-level | up | 2.831 s (p95 3.071, sd 0.854) | 8.510 s (p95 8.656, sd 0.131) | 7.321 s (p95 9.678, sd 0.721) |
| wide-level | down | 5.155 s (p95 6.275, sd 1.319) | 7.429 s (p95 9.890, sd 0.983) | 2.536 s (p95 2.762, sd 0.107) |
| scale | up | 0.432 s (p95 0.877, sd 0.180) | 0.477 s (p95 0.493, sd 0.010) | 0.981 s (p95 1.011, sd 0.015) |
| scale | down | 0.689 s (p95 1.762, sd 0.329) | 0.420 s (p95 0.427, sd 0.010) | 0.525 s (p95 0.552, sd 0.015) |
| network-ipam | up | 204.5 ms (p95 216.5, sd 6.5) | 679.7 ms (p95 700.5, sd 16.8) | 438.5 ms (p95 448.1, sd 6.7) |
| network-ipam | down | 255.6 ms (p95 276.7, sd 9.4) | 594.1 ms (p95 643.7, sd 19.5) | 344.2 ms (p95 362.6, sd 13.0) |
| volume-heavy | up | 208.0 ms (p95 221.3, sd 8.3) | 948.3 ms (p95 963.2, sd 6.2) | 277.6 ms (p95 289.7, sd 5.9) |
| volume-heavy | down | 264.2 ms (p95 291.4, sd 13.4) | 665.6 ms (p95 680.1, sd 8.6) | 306.6 ms (p95 317.5, sd 12.8) |
| secrets | up | 164.9 ms (p95 196.4, sd 14.4) | 493.0 ms (p95 501.4, sd 7.3) | 275.9 ms (p95 288.3, sd 5.4) |
| secrets | down | 219.2 ms (p95 249.2, sd 14.0) | 476.4 ms (p95 487.9, sd 9.3) | 295.3 ms (p95 312.6, sd 7.4) |
| warm-restart | warm up | 45.2 ms (p95 63.4, sd 9.0) | 262.9 ms (p95 273.0, sd 6.0) | 54.0 ms (p95 65.1, sd 3.8) |
| many-services | up | 0.964 s (p95 1.073, sd 0.095) | 2.560 s (p95 2.645, sd 0.044) | 2.152 s (p95 2.336, sd 0.070) |
| many-services | down | 1.518 s (p95 1.731, sd 0.092) | 2.182 s (p95 2.328, sd 0.084) | 0.916 s (p95 0.976, sd 0.035) |
| running-ops | ps | 5.6 ms (p95 7.3, sd 0.6) | 115.2 ms (p95 117.6, sd 1.2) | 26.4 ms (p95 28.9, sd 0.9) |
| running-ops | logs | 5.5 ms (p95 6.7, sd 0.4) | 159.2 ms (p95 169.7, sd 4.5) | 26.3 ms (p95 34.7, sd 2.6) |
| running-ops | exec | 82.8 ms (p95 88.3, sd 3.6) | 234.5 ms (p95 243.8, sd 4.1) | 41.1 ms (p95 46.0, sd 1.8) |
| running-ops | restart | 231.7 ms (p95 252.0, sd 10.2) | 319.9 ms (p95 354.0, sd 13.9) | 276.8 ms (p95 303.2, sd 10.4) |
| wide-running-ops | ps | 9.7 ms (p95 11.3, sd 0.8) | 182.5 ms (p95 193.1, sd 3.5) | 50.4 ms (p95 54.4, sd 1.5) |
| wide-running-ops | logs | 9.3 ms (p95 9.8, sd 0.7) | 212.1 ms (p95 215.8, sd 3.0) | 29.4 ms (p95 30.9, sd 0.8) |
| wide-running-ops | exec | 90.6 ms (p95 100.2, sd 4.7) | 280.8 ms (p95 285.8, sd 4.0) | 41.6 ms (p95 42.8, sd 1.0) |
| wide-running-ops | restart | 174.7 ms (p95 192.6, sd 8.0) | 334.8 ms (p95 345.9, sd 5.3) | 289.1 ms (p95 305.3, sd 8.2) |
| config-heavy | config | 12.0 ms (p95 17.7, sd 1.7) | 538.3 ms (p95 549.6, sd 4.3) | 33.7 ms (p95 35.0, sd 0.7) |
| build | build | 0.437 s (p95 0.440, sd 0.004) | 0.470 s (p95 0.487, sd 0.030) | 0.933 s (p95 2.511, sd 0.495) |

## Memory + CPU per command, each tool on its own engine

| scenario | op | podup | podman-compose | docker-compose (Docker) |
|---|---|---|---|---|
| single | up | 7.2 MiB / 5.9 ms | 51.5 MiB / 471.6 ms | 29.9 MiB / 34.5 ms |
| single | down | 7.7 MiB / 7.4 ms | 50.0 MiB / 351.0 ms | 29.2 MiB / 32.0 ms |
| multi-healthcheck | up | 7.2 MiB / 7.7 ms | 52.0 MiB / 727.4 ms | 29.8 MiB / 38.8 ms |
| multi-healthcheck | down | 7.6 MiB / 8.0 ms | 50.3 MiB / 478.1 ms | 29.2 MiB / 33.8 ms |
| deep-chain | up | 7.3 MiB / 0.009 s | 52.5 MiB / 1.334 s | 30.1 MiB / 0.043 s |
| deep-chain | down | 7.7 MiB / 9.9 ms | 50.5 MiB / 846.6 ms | 29.4 MiB / 36.3 ms |
| wide-level | up | 8.0 MiB / 0.031 s | 53.1 MiB / 7.003 s | 34.2 MiB / 0.091 s |
| wide-level | down | 8.0 MiB / 0.026 s | 51.4 MiB / 5.451 s | 31.4 MiB / 0.067 s |
| scale | up | 7.3 MiB / 8.0 ms | 51.6 MiB / 498.0 ms | 30.3 MiB / 40.6 ms |
| scale | down | 7.8 MiB / 8.9 ms | 50.0 MiB / 347.2 ms | 29.5 MiB / 34.7 ms |
| network-ipam | up | 7.2 MiB / 6.7 ms | 52.1 MiB / 625.9 ms | 29.9 MiB / 36.9 ms |
| network-ipam | down | 7.7 MiB / 7.9 ms | 50.1 MiB / 476.0 ms | 29.3 MiB / 33.0 ms |
| volume-heavy | up | 7.2 MiB / 0.007 s | 52.2 MiB / 1.054 s | 30.4 MiB / 0.039 s |
| volume-heavy | down | 7.8 MiB / 8.5 ms | 49.6 MiB / 571.1 ms | 29.9 MiB / 37.1 ms |
| secrets | up | 7.2 MiB / 7.7 ms | 51.3 MiB / 474.0 ms | 29.9 MiB / 35.8 ms |
| secrets | down | 7.8 MiB / 8.3 ms | 49.8 MiB / 368.4 ms | 29.2 MiB / 32.7 ms |
| warm-restart | warm up | 7.2 MiB / 6.7 ms | 47.1 MiB / 314.1 ms | 29.8 MiB / 34.4 ms |
| many-services | up | 7.5 MiB / 0.012 s | 53.1 MiB / 2.209 s | 31.3 MiB / 0.052 s |
| many-services | down | 7.9 MiB / 0.012 s | 50.8 MiB / 1.651 s | 30.2 MiB / 0.043 s |
| running-ops | ps | 6.9 MiB / 4.2 ms | 46.5 MiB / 129.0 ms | 29.1 MiB / 29.6 ms |
| running-ops | logs | 6.9 MiB / 4.2 ms | 62.1 MiB / 139.4 ms | 29.1 MiB / 30.0 ms |
| running-ops | exec | 6.9 MiB / 4.8 ms | 47.2 MiB / 142.0 ms | 27.3 MiB / 17.2 ms |
| running-ops | restart | 6.9 MiB / 4.7 ms | 47.9 MiB / 178.7 ms | 29.4 MiB / 31.2 ms |
| wide-running-ops | ps | 6.9 MiB / 5.3 ms | 47.8 MiB / 201.0 ms | 30.7 MiB / 42.7 ms |
| wide-running-ops | logs | 6.9 MiB / 5.2 ms | 62.4 MiB / 204.8 ms | 29.7 MiB / 34.2 ms |
| wide-running-ops | exec | 7.0 MiB / 5.7 ms | 47.3 MiB / 204.8 ms | 27.3 MiB / 18.2 ms |
| wide-running-ops | restart | 6.9 MiB / 5.6 ms | 48.0 MiB / 236.1 ms | 29.7 MiB / 35.8 ms |
| config-heavy | config | 7.4 MiB / 12.7 ms | 34.5 MiB / 542.0 ms | 30.5 MiB / 49.1 ms |
| build | build | 7.5 MiB / 5.4 ms | 61.6 MiB / 381.7 ms | 51.3 MiB / 126.5 ms |

## Reading these numbers honestly

On the same engine podup is fastest in 25 of 29 rows. Of the four it does not
take, three are ties and one is a real loss:

| row | podup | best of the others | gap | podup's own sd |
|---|---|---|---|---|
| running-ops restart | 231.7 ms | docker-compose 231.3 ms | 0.5 ms | 10.2 ms |
| secrets down | 219.2 ms | docker-compose 215.5 ms | 3.7 ms | 14.0 ms |
| secrets up | 164.9 ms | docker-compose 147.8 ms | 17.1 ms | 14.4 ms |
| scale down | 689.3 ms | podman-compose 419.8 ms | 269.5 ms | 328.7 ms |

`secrets up` is about one standard deviation, so read it as a narrow docker-compose
lead rather than a tie. `scale down` is inside podup's own spread in this run, but
four head-to-head rounds on the same host gave podup 0.63 to 0.87 s against
podman-compose 0.41 to 0.48 s every time, with the 5.7.1 binary as slow as 5.10.5,
so it is a real gap and not a regression (#1947). Of podup's wins, five clear the
bar by less than two of its standard deviations and are better read as "about
the same": `many-services down`, `scale up`, `warm-restart warm up`, `wide-level
down` and `wide-running-ops exec`.

Against Docker on its own engine, podup is fastest in 24 of 29 rows. dockerd wins
the teardown of many containers (`wide-level down` 2.54 s against 5.16 s,
`many-services down` 0.92 s against 1.52 s) and `exec` (41 ms against 83 ms).
Those are the engine: docker-compose on the Podman socket is as slow as podup on
the same rows.

**Memory grew.** podup's peak per command is a median of 7.3 MiB (worst 8.0,
`wide-level up`), against 5.6 MiB (worst 6.5) in the 5.7.1 run. The head-to-head
puts most of it at startup: `podup --version` peaks at 3452 KB on 5.7.1 and
5296 KB on 5.10.5. It is under the 9.0 MiB budget in `bench/memory-budget-mib`,
and the attribution is #1946.

podman-compose moved the most between versions: `config-heavy config` is 538 ms
here with 1.6.0 against 113 ms with 1.5.0 in the previous run, on a row that does
not touch the engine at all.

`multi-healthcheck up` still measures healthcheck interval granularity more than
tool speed, and the `secrets` rows still carry the three API calls per secret at
`up` and two at `down` that native Podman secrets cost since 3.1.0.
