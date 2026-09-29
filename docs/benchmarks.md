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
run had one: `scale down` for podman-compose failed all 10 iterations (the tool
exited 0 but left four replicas, the pod and the network behind every time,
so the harness rewrote it to rc=97; see `bench/leftovers.sh` and #1947).
1044 timed runs, 10 failed.

Reproduce with `bash bench/run.sh`, then `python3 bench/aggregate.py`; the harness
measures both docker-compose variants when both engines answer, and writes the
host description below into `bench/results/env.txt`. Timing comes from
`bench/timeit` (fork, clock across the command, peak RSS and CPU from `wait4`'s
rusage).

Each row carries **one unit**, picked from the largest value in that row and
applied to every tool in it. `bench/results/raw.csv` and `summary.json` keep every
figure in seconds.

Measured on podup **5.10.8**, the published `podup-linux-x86_64` asset (static
musl, the binary the installers fetch, checked against `SHA256SUMS`):

```
kernel: 7.0.0-34-generic
cpu: AMD Ryzen 7 5700X 8-Core Processor (16 threads), governor performance, tools pinned to cores 2-9
podman: 5.7.0 (Ubuntu 26.04)
podman-compose: 1.6.0
docker-compose: 5.5.1
docker server: not running on this host, so the cross-engine comparison was not measured this run
podman store at start: 41 containers, 44 networks, 99 volumes, 442 images, 1 dangling
running VMs: 0
```

**Do not compare these numbers across runs.** The engine on this host was
two to three times slower on the multi-container rows in the 5.10.5 run the page
used to carry than in the 5.7.1 run it replaced, and that did not bear on the
within-run comparison; the 5.10.8 run is faster than 5.10.5 on those same rows,
and the gap is not attributed. Comparing the 5.10.8 multi-container rows against
the 5.10.5 numbers the page currently shows, `wide-level up` for podup went from
2.83 s in 5.10.5 to 1.13 s here (about 2.5 times faster), `wide-level down`
from 5.16 s to 1.63 s (about 3.2 times faster), and the podman-compose
`wide-level up` from 8.51 s to 7.26 s. The cause of either move is not
identified; the warning not to read numbers across runs stands, and every
figure below describes the 5.10.8 run on its own.

## Wall-clock, same engine (lower is better)

| scenario | op | podup | podman-compose | docker-compose (Podman) |
|---|---|---|---|---|
| single | up | 92.4 ms (p95 122.1, sd 10.2) | 410.1 ms (p95 427.3, sd 9.7) | 116.5 ms (p95 138.1, sd 8.3) |
| single | down | 140.9 ms (p95 155.2, sd 10.4) | 387.8 ms (p95 411.7, sd 15.1) | 169.8 ms (p95 184.1, sd 9.5) |
| multi-healthcheck | up | 0.363 s (p95 0.404, sd 0.065) | 0.906 s (p95 1.021, sd 0.042) | 0.723 s (p95 0.751, sd 0.015) |
| multi-healthcheck | down | 272.7 ms (p95 318.2, sd 21.6) | 561.4 ms (p95 648.4, sd 43.9) | 276.9 ms (p95 306.2, sd 15.2) |
| deep-chain | up | 0.345 s (p95 0.380, sd 0.018) | 1.604 s (p95 1.651, sd 0.021) | 0.857 s (p95 0.910, sd 0.021) |
| deep-chain | down | 404.0 ms (p95 455.2, sd 19.9) | 820.5 ms (p95 858.0, sd 31.2) | 414.8 ms (p95 437.7, sd 13.8) |
| wide-level | up | 1.133 s (p95 1.205, sd 0.036) | 7.264 s (p95 7.338, sd 0.051) | 2.937 s (p95 3.027, sd 0.048) |
| wide-level | down | 1.632 s (p95 2.138, sd 0.190) | 4.542 s (p95 4.714, sd 0.103) | 2.071 s (p95 2.883, sd 0.382) |
| scale | up | 0.193 s (p95 0.204, sd 0.011) | 1.054 s (p95 1.306, sd 0.080) | 0.392 s (p95 0.409, sd 0.012) |
| scale | down | 276.5 ms (p95 348.4, sd 29.2) | [10 failed of 10] | 291.2 ms (p95 337.4, sd 22.1) |
| network-ipam | up | 109.9 ms (p95 125.8, sd 6.1) | 591.5 ms (p95 634.5, sd 17.4) | 186.0 ms (p95 200.4, sd 8.9) |
| network-ipam | down | 173.1 ms (p95 195.0, sd 13.6) | 504.7 ms (p95 528.1, sd 17.5) | 205.3 ms (p95 230.1, sd 21.6) |
| volume-heavy | up | 107.6 ms (p95 116.5, sd 4.8) | 913.5 ms (p95 958.8, sd 16.0) | 148.4 ms (p95 184.5, sd 13.8) |
| volume-heavy | down | 153.4 ms (p95 177.2, sd 11.6) | 585.7 ms (p95 616.0, sd 10.6) | 202.8 ms (p95 222.5, sd 10.6) |
| secrets | up | 110.9 ms (p95 116.4, sd 4.9) | 441.1 ms (p95 477.8, sd 16.4) | 121.7 ms (p95 137.2, sd 6.1) |
| secrets | down | 152.4 ms (p95 166.7, sd 9.1) | 406.4 ms (p95 427.5, sd 13.3) | 169.4 ms (p95 190.8, sd 11.0) |
| warm-restart | warm up | 37.8 ms (p95 47.4, sd 4.1) | 246.5 ms (p95 255.0, sd 5.5) | 53.5 ms (p95 58.7, sd 6.3) |
| many-services | up | 0.398 s (p95 0.414, sd 0.014) | 2.220 s (p95 2.326, sd 0.045) | 0.884 s (p95 0.905, sd 0.017) |
| many-services | down | 0.562 s (p95 0.664, sd 0.072) | 1.391 s (p95 1.455, sd 0.031) | 0.552 s (p95 0.641, sd 0.037) |
| running-ops | ps | 7.5 ms (p95 8.4, sd 0.5) | 119.0 ms (p95 122.4, sd 2.0) | 27.2 ms (p95 28.1, sd 0.6) |
| running-ops | logs | 9.3 ms (p95 12.3, sd 1.3) | 148.8 ms (p95 157.0, sd 4.6) | 39.4 ms (p95 45.0, sd 2.9) |
| running-ops | exec | 65.7 ms (p95 69.1, sd 3.0) | 202.1 ms (p95 218.4, sd 5.7) | 76.1 ms (p95 84.3, sd 5.8) |
| running-ops | restart | 167.8 ms (p95 189.9, sd 11.2) | 298.0 ms (p95 317.4, sd 15.2) | 186.5 ms (p95 209.7, sd 9.4) |
| wide-running-ops | ps | 11.7 ms (p95 12.8, sd 0.7) | 190.7 ms (p95 200.7, sd 5.5) | 43.0 ms (p95 44.9, sd 0.7) |
| wide-running-ops | logs | 11.7 ms (p95 14.6, sd 1.4) | 205.5 ms (p95 213.3, sd 5.1) | 45.0 ms (p95 50.3, sd 2.7) |
| wide-running-ops | exec | 66.9 ms (p95 76.6, sd 5.4) | 250.6 ms (p95 255.2, sd 2.7) | 78.0 ms (p95 85.7, sd 3.8) |
| wide-running-ops | restart | 128.5 ms (p95 149.1, sd 10.7) | 296.3 ms (p95 305.5, sd 5.0) | 149.1 ms (p95 163.2, sd 5.4) |
| config-heavy | config | 12.3 ms (p95 16.7, sd 1.3) | 542.3 ms (p95 553.2, sd 5.1) | 37.8 ms (p95 41.5, sd 1.8) |
| build | build | 0.283 s (p95 0.293, sd 0.004) | 0.368 s (p95 0.393, sd 0.010) | 1.205 s (p95 1.642, sd 0.146) |

> **rc=97** (10 rows): Rows with rc=97 exited cleanly but left containers, pods or networks of their compose project behind; the harness rewrote them to 97 so a clean teardown is not compared against one that did a fraction of it. See bench/leftovers.sh for the detection and force-purge.

## Memory + CPU per command, same engine (peak RSS / CPU time, median)

Client-side cost of invoking the tool: the tool process and what it spawns and
waits on. podup is a static binary talking to the Podman service; podman-compose
is Python shelling out to `podman` per call and is charged for that work.

| scenario | op | podup | podman-compose | docker-compose (Podman) |
|---|---|---|---|---|
| single | up | 6.0 MiB / 7.0 ms | 52.1 MiB / 479.6 ms | 29.9 MiB / 35.4 ms |
| single | down | 6.3 MiB / 8.0 ms | 50.6 MiB / 368.7 ms | 29.6 MiB / 32.9 ms |
| multi-healthcheck | up | 5.9 MiB / 8.4 ms | 52.6 MiB / 738.0 ms | 30.2 MiB / 38.4 ms |
| multi-healthcheck | down | 6.2 MiB / 8.5 ms | 51.1 MiB / 499.5 ms | 29.4 MiB / 34.8 ms |
| deep-chain | up | 6.1 MiB / 0.010 s | 52.9 MiB / 1.360 s | 30.2 MiB / 0.043 s |
| deep-chain | down | 6.3 MiB / 10.6 ms | 51.3 MiB / 870.2 ms | 29.4 MiB / 37.4 ms |
| wide-level | up | 6.9 MiB / 0.030 s | 53.4 MiB / 7.262 s | 34.0 MiB / 0.093 s |
| wide-level | down | 6.7 MiB / 0.026 s | 51.6 MiB / 5.538 s | 31.9 MiB / 0.070 s |
| scale | up | 6.1 MiB / 0.009 s | 52.7 MiB / 1.115 s | 30.1 MiB / 0.041 s |
| scale | down | 6.3 MiB / 9.5 ms | [10 failed of 10] | 29.5 MiB / 36.2 ms |
| network-ipam | up | 6.1 MiB / 7.4 ms | 52.5 MiB / 653.1 ms | 29.8 MiB / 37.5 ms |
| network-ipam | down | 6.3 MiB / 8.4 ms | 50.6 MiB / 495.1 ms | 29.1 MiB / 34.0 ms |
| volume-heavy | up | 6.0 MiB / 0.008 s | 52.4 MiB / 1.104 s | 30.4 MiB / 0.041 s |
| volume-heavy | down | 6.3 MiB / 9.1 ms | 50.3 MiB / 589.6 ms | 30.1 MiB / 38.3 ms |
| secrets | up | 6.1 MiB / 8.7 ms | 52.0 MiB / 497.9 ms | 29.9 MiB / 36.7 ms |
| secrets | down | 6.3 MiB / 8.7 ms | 50.8 MiB / 385.1 ms | 29.4 MiB / 32.8 ms |
| warm-restart | warm up | 6.0 MiB / 7.5 ms | 49.2 MiB / 326.3 ms | 30.0 MiB / 35.2 ms |
| many-services | up | 6.3 MiB / 0.013 s | 53.3 MiB / 2.279 s | 31.5 MiB / 0.054 s |
| many-services | down | 6.4 MiB / 0.013 s | 51.3 MiB / 1.682 s | 30.2 MiB / 0.044 s |
| running-ops | ps | 5.4 MiB / 4.3 ms | 49.0 MiB / 136.7 ms | 29.4 MiB / 29.7 ms |
| running-ops | logs | 5.7 MiB / 4.6 ms | 68.5 MiB / 141.7 ms | 29.2 MiB / 31.8 ms |
| running-ops | exec | 5.7 MiB / 5.4 ms | 48.3 MiB / 140.9 ms | 27.2 MiB / 19.8 ms |
| running-ops | restart | 5.8 MiB / 5.2 ms | 48.9 MiB / 182.6 ms | 29.6 MiB / 32.4 ms |
| wide-running-ops | ps | 5.5 MiB / 5.4 ms | 50.2 MiB / 206.7 ms | 30.4 MiB / 42.6 ms |
| wide-running-ops | logs | 5.7 MiB / 5.4 ms | 68.4 MiB / 205.7 ms | 29.5 MiB / 36.8 ms |
| wide-running-ops | exec | 5.7 MiB / 6.2 ms | 48.3 MiB / 205.0 ms | 27.3 MiB / 19.8 ms |
| wide-running-ops | restart | 5.9 MiB / 6.2 ms | 49.1 MiB / 241.1 ms | 30.2 MiB / 37.0 ms |
| config-heavy | config | 6.0 MiB / 12.8 ms | 34.8 MiB / 545.1 ms | 30.8 MiB / 52.9 ms |
| build | build | 6.0 MiB / 6.2 ms | 64.7 MiB / 397.6 ms | 60.7 MiB / 190.7 ms |

## Wall-clock, each tool on its own engine

This table is from the **5.10.5 run on 2026-09-27** and was not re-measured for
5.10.8 because the Docker daemon was not running on the benchmark host.
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

This table is from the **5.10.5 run on 2026-09-27** and was not re-measured for
5.10.8 because the Docker daemon was not running on the benchmark host.

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

On the same engine podup is fastest in **28 of 29 rows**. The one row it does
not take is `many-services down`, where docker-compose beats it by 10.1 ms
against podup's own standard deviation of 72.2 ms (about 0.14 sd), well inside
podup's spread:

| row | podup | best of the others | gap | podup's own sd |
|---|---|---|---|---|
| many-services down | 562.0 ms | docker-compose 551.9 ms | 10.1 ms | 72.2 ms |

On a second row, `scale down` for podman-compose, the harness refused the cell
after podman-compose 1.6.0 exited 0 from `down -v` but left `app_2` through
`app_5`, the pod and the network of its compose project behind, on all 10
iterations (#1947); podup's own number on that row stands on its own.

Of podup's wins, six clear the bar by less than two of its standard deviations
and are better read as "about the same": `multi-healthcheck down` (0.20 sd),
`deep-chain down` (0.55 sd), `scale down` (0.51 sd), `secrets down` (1.87 sd),
`running-ops restart` (1.67 sd) and `wide-running-ops restart` (1.93 sd).

Against Docker on its own engine, the comparison comes from the **5.10.5 run**
(below), since the Docker daemon was not running on this host when 5.10.8 was
measured. In that table podup is fastest in 24 of 29 rows; dockerd wins the
teardown of many containers (`wide-level down` 2.54 s against 5.16 s,
`many-services down` 0.92 s against 1.52 s) and `exec` (41 ms against 83 ms).
Those are the engine: docker-compose on the Podman socket is as slow as podup on
the same rows.

**Memory.** podup's peak per command is a median of 6.1 MiB (worst 6.9,
`wide-level up`). It is under the 9.0 MiB budget in `bench/memory-budget-mib`.
#1946 did not reproduce when measured again: `podup --version` peaks at 3.3 to 3.7 MB
on 5.7.1, 5.10.5 and 5.10.7 alike.

podman-compose `config-heavy config` is 542 ms here with 1.6.0, on a row that does
not touch the engine at all, against 113 ms with 1.5.0 in the previous run; the
regression persists.

`multi-healthcheck up` still measures healthcheck interval granularity more than
tool speed, and the `secrets` rows still carry the three API calls per secret at
`up` and two at `down` that native Podman secrets cost since 3.1.0.
