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
run had one: `scale down` for podman-compose failed all 10 measured iterations
(the tool exited 0 but left four replicas, the pod and the network behind every
time, so the harness rewrote it to rc=97; see `bench/leftovers.sh` and #1947).
1392 timed runs, 10 with rc=97.

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
date: 2026-09-29T14:33:45Z
kernel: 7.0.0-34-generic
cpu: AMD Ryzen 7 5700X 8-Core Processor
cores: 16
governor: performance
pinned_cores: 2-9
podup: podup version v5.10.8
podup_linkage: static-pie linked
podman: podman version 5.7.0
podman-compose: podman-compose version 1.6.0
docker_compose: 5.5.1
docker server: 29.8.1
podman_containers: 41
podman_networks: 44
podman_volumes: 99
podman_images: 442
podman_dangling_images: 1
docker_containers: 0
docker_images: 5
running_vms: 0
```

**Do not compare these numbers across runs.** The engine on this host was
two to three times slower on the multi-container rows in the 5.10.5 run the
page used to carry than in the 5.10.8 run it replaced, and that did not bear
on the within-run comparison; the 5.10.8 numbers here are themselves a fresh
re-run of the same published 5.10.8 asset with the Docker daemon started and
both engines measured, and the cause of either move is not identified.
Comparing the 5.10.8 multi-container rows against the 5.10.5 numbers the page
used to carry, `wide-level up` for podup went from 2.83 s in 5.10.5 to 1.13 s
here (about 2.5 times faster), `wide-level down` from 5.16 s to 1.76 s (about
2.9 times faster), and the podman-compose `wide-level up` from 8.51 s to 7.32 s.
The cause of either move is not identified; the warning not to read numbers
across runs stands, and every figure below describes this 5.10.8 run on its own.

## Wall-clock, same engine (lower is better)

| scenario | op | podup | podman-compose | docker-compose (Podman) |
|---|---|---|---|---|
| single | up | 92.4 ms (p95 103.6, sd 4.9) | 417.4 ms (p95 429.1, sd 9.4) | 118.4 ms (p95 141.5, sd 8.3) |
| single | down | 148.4 ms (p95 161.4, sd 8.1) | 388.4 ms (p95 408.8, sd 11.8) | 161.8 ms (p95 180.9, sd 9.8) |
| multi-healthcheck | up | 0.400 s (p95 0.594, sd 0.113) | 0.922 s (p95 2.611, sd 0.516) | 0.772 s (p95 0.921, sd 0.062) |
| multi-healthcheck | down | 282.9 ms (p95 636.4, sd 111.6) | 593.7 ms (p95 824.8, sd 92.4) | 413.7 ms (p95 627.5, sd 122.7) |
| deep-chain | up | 0.359 s (p95 0.388, sd 0.021) | 1.568 s (p95 1.630, sd 0.079) | 0.852 s (p95 0.967, sd 0.039) |
| deep-chain | down | 403.2 ms (p95 448.9, sd 17.4) | 810.9 ms (p95 841.8, sd 15.7) | 403.7 ms (p95 423.2, sd 11.1) |
| wide-level | up | 1.132 s (p95 1.259, sd 0.050) | 7.319 s (p95 9.284, sd 0.593) | 2.971 s (p95 3.122, sd 0.067) |
| wide-level | down | 1.757 s (p95 2.022, sd 0.149) | 4.875 s (p95 6.132, sd 0.516) | 2.013 s (p95 3.037, sd 0.441) |
| scale | up | 0.197 s (p95 0.228, sd 0.015) | 1.085 s (p95 1.107, sd 0.023) | 0.385 s (p95 0.423, sd 0.017) |
| scale | down | 269.1 ms (p95 307.3, sd 16.8) | [10 failed of 10] | 327.7 ms (p95 384.4, sd 37.8) |
| network-ipam | up | 110.4 ms (p95 133.0, sd 9.0) | 595.3 ms (p95 690.9, sd 33.4) | 186.0 ms (p95 207.6, sd 9.7) |
| network-ipam | down | 175.9 ms (p95 202.2, sd 13.1) | 509.2 ms (p95 550.5, sd 21.5) | 194.0 ms (p95 223.8, sd 13.7) |
| volume-heavy | up | 105.0 ms (p95 122.8, sd 7.5) | 920.9 ms (p95 944.0, sd 15.0) | 159.0 ms (p95 177.4, sd 12.5) |
| volume-heavy | down | 156.3 ms (p95 165.4, sd 6.0) | 583.3 ms (p95 635.5, sd 21.1) | 207.6 ms (p95 246.8, sd 15.2) |
| secrets | up | 107.3 ms (p95 122.5, sd 5.8) | 471.8 ms (p95 489.3, sd 11.5) | 128.4 ms (p95 145.6, sd 9.8) |
| secrets | down | 159.5 ms (p95 167.3, sd 6.7) | 430.1 ms (p95 449.3, sd 13.4) | 181.9 ms (p95 201.4, sd 10.1) |
| warm-restart | warm up | 42.0 ms (p95 46.8, sd 3.5) | 262.1 ms (p95 273.9, sd 9.4) | 53.4 ms (p95 65.2, sd 6.4) |
| many-services | up | 0.393 s (p95 0.422, sd 0.018) | 2.259 s (p95 2.344, sd 0.039) | 0.899 s (p95 0.993, sd 0.048) |
| many-services | down | 0.496 s (p95 0.620, sd 0.044) | 1.421 s (p95 1.552, sd 0.058) | 0.568 s (p95 0.625, sd 0.038) |
| running-ops | ps | 7.9 ms (p95 9.4, sd 0.8) | 120.9 ms (p95 127.2, sd 2.4) | 29.3 ms (p95 76.9, sd 17.5) |
| running-ops | logs | 8.2 ms (p95 12.2, sd 1.4) | 149.4 ms (p95 151.4, sd 2.7) | 46.7 ms (p95 72.4, sd 10.5) |
| running-ops | exec | 64.2 ms (p95 73.9, sd 5.4) | 202.8 ms (p95 208.5, sd 6.8) | 84.4 ms (p95 135.6, sd 18.5) |
| running-ops | restart | 173.3 ms (p95 191.5, sd 9.5) | 304.4 ms (p95 342.9, sd 20.1) | 195.6 ms (p95 297.7, sd 37.7) |
| wide-running-ops | ps | 11.6 ms (p95 12.7, sd 0.6) | 191.0 ms (p95 196.8, sd 3.8) | 44.6 ms (p95 91.8, sd 14.5) |
| wide-running-ops | logs | 11.3 ms (p95 15.7, sd 1.9) | 206.5 ms (p95 214.5, sd 4.9) | 44.6 ms (p95 50.4, sd 2.8) |
| wide-running-ops | exec | 68.8 ms (p95 74.0, sd 3.8) | 252.0 ms (p95 264.9, sd 5.5) | 79.7 ms (p95 95.0, sd 5.7) |
| wide-running-ops | restart | 123.3 ms (p95 133.3, sd 4.3) | 302.4 ms (p95 311.6, sd 6.7) | 161.7 ms (p95 269.9, sd 33.1) |
| config-heavy | config | 13.3 ms (p95 14.6, sd 0.9) | 554.7 ms (p95 564.2, sd 5.2) | 115.3 ms (p95 131.6, sd 11.3) |
| build | build | 0.285 s (p95 0.306, sd 0.008) | 0.368 s (p95 0.415, sd 0.015) | 1.469 s (p95 2.139, sd 0.270) |

> **rc=97** (10 rows): Rows with rc=97 exited cleanly but left containers, pods or networks of their compose project behind; the harness rewrote them to 97 so a clean teardown is not compared against one that did a fraction of it. See bench/leftovers.sh for the detection and force-purge.

## Memory + CPU per command, same engine (peak RSS / CPU time, median)

Client-side cost of invoking the tool: the tool process and what it spawns and
waits on. podup is a static binary talking to the Podman service; podman-compose
is Python shelling out to `podman` per call and is charged for that work.

| scenario | op | podup | podman-compose | docker-compose (Podman) |
|---|---|---|---|---|
| single | up | 6.0 MiB / 7.0 ms | 52.5 MiB / 475.4 ms | 29.7 MiB / 35.3 ms |
| single | down | 6.3 MiB / 8.0 ms | 50.8 MiB / 365.9 ms | 29.3 MiB / 32.4 ms |
| multi-healthcheck | up | 6.0 MiB / 10.2 ms | 52.6 MiB / 736.5 ms | 30.0 MiB / 39.1 ms |
| multi-healthcheck | down | 6.2 MiB / 10.1 ms | 51.1 MiB / 507.4 ms | 29.8 MiB / 34.6 ms |
| deep-chain | up | 6.2 MiB / 0.010 s | 53.0 MiB / 1.363 s | 30.3 MiB / 0.044 s |
| deep-chain | down | 6.3 MiB / 10.9 ms | 51.2 MiB / 874.4 ms | 29.6 MiB / 37.3 ms |
| wide-level | up | 6.9 MiB / 0.031 s | 53.5 MiB / 7.310 s | 34.6 MiB / 0.096 s |
| wide-level | down | 6.7 MiB / 0.026 s | 51.6 MiB / 5.569 s | 31.9 MiB / 0.071 s |
| scale | up | 6.1 MiB / 0.009 s | 52.9 MiB / 1.122 s | 30.0 MiB / 0.041 s |
| scale | down | 6.3 MiB / 9.6 ms | [10 failed of 10] | 29.6 MiB / 36.1 ms |
| network-ipam | up | 6.0 MiB / 7.6 ms | 52.5 MiB / 657.9 ms | 30.1 MiB / 38.8 ms |
| network-ipam | down | 6.2 MiB / 8.7 ms | 50.7 MiB / 495.3 ms | 29.4 MiB / 34.0 ms |
| volume-heavy | up | 6.1 MiB / 0.008 s | 52.4 MiB / 1.110 s | 30.3 MiB / 0.042 s |
| volume-heavy | down | 6.3 MiB / 9.0 ms | 50.5 MiB / 591.7 ms | 30.0 MiB / 38.0 ms |
| secrets | up | 6.0 MiB / 8.6 ms | 52.2 MiB / 499.6 ms | 30.0 MiB / 37.8 ms |
| secrets | down | 6.3 MiB / 8.9 ms | 50.8 MiB / 386.3 ms | 29.4 MiB / 35.3 ms |
| warm-restart | warm up | 6.0 MiB / 7.7 ms | 49.4 MiB / 328.6 ms | 30.1 MiB / 37.1 ms |
| many-services | up | 6.3 MiB / 0.013 s | 53.2 MiB / 2.281 s | 31.5 MiB / 0.055 s |
| many-services | down | 6.4 MiB / 0.013 s | 51.3 MiB / 1.696 s | 30.3 MiB / 0.046 s |
| running-ops | ps | 5.5 MiB / 4.4 ms | 49.2 MiB / 137.5 ms | 29.3 MiB / 31.9 ms |
| running-ops | logs | 5.8 MiB / 4.6 ms | 68.6 MiB / 143.3 ms | 29.3 MiB / 34.5 ms |
| running-ops | exec | 5.6 MiB / 5.4 ms | 48.0 MiB / 143.8 ms | 27.4 MiB / 20.9 ms |
| running-ops | restart | 5.8 MiB / 5.3 ms | 48.9 MiB / 184.0 ms | 29.6 MiB / 34.8 ms |
| wide-running-ops | ps | 5.4 MiB / 5.3 ms | 49.9 MiB / 208.0 ms | 30.8 MiB / 42.2 ms |
| wide-running-ops | logs | 5.7 MiB / 5.4 ms | 68.6 MiB / 207.5 ms | 29.6 MiB / 38.7 ms |
| wide-running-ops | exec | 5.6 MiB / 6.3 ms | 48.1 MiB / 207.5 ms | 27.3 MiB / 20.1 ms |
| wide-running-ops | restart | 5.8 MiB / 6.2 ms | 49.2 MiB / 242.8 ms | 30.0 MiB / 39.7 ms |
| config-heavy | config | 6.0 MiB / 13.1 ms | 34.9 MiB / 556.6 ms | 33.2 MiB / 117.9 ms |
| build | build | 6.0 MiB / 6.0 ms | 65.6 MiB / 402.8 ms | 61.1 MiB / 202.5 ms |

## Wall-clock, each tool on its own engine

podup and podman-compose drive rootless Podman; docker-compose drives
the Docker daemon (rootful). This is what a user of each stack sees, and the
engines differ, so it is not a pure tool comparison: docker-compose runs
against dockerd, not the Podman socket, and engine differences are folded into
its column.

| scenario | op | podup | podman-compose | docker-compose (Docker) |
|---|---|---|---|---|
| single | up | 92.4 ms (p95 103.6, sd 4.9) | 417.4 ms (p95 429.1, sd 9.4) | 201.7 ms (p95 382.6, sd 56.0) |
| single | down | 148.4 ms (p95 161.4, sd 8.1) | 388.4 ms (p95 408.8, sd 11.8) | 232.7 ms (p95 311.9, sd 25.9) |
| multi-healthcheck | up | 0.400 s (p95 0.594, sd 0.113) | 0.922 s (p95 2.611, sd 0.516) | 1.361 s (p95 1.736, sd 0.122) |
| multi-healthcheck | down | 282.9 ms (p95 636.4, sd 111.6) | 593.7 ms (p95 824.8, sd 92.4) | 425.6 ms (p95 650.5, sd 70.8) |
| deep-chain | up | 0.359 s (p95 0.388, sd 0.021) | 1.568 s (p95 1.630, sd 0.079) | 2.571 s (p95 2.785, sd 0.216) |
| deep-chain | down | 0.403 s (p95 0.449, sd 0.017) | 0.811 s (p95 0.842, sd 0.016) | 0.777 s (p95 1.176, sd 0.186) |
| wide-level | up | 1.132 s (p95 1.259, sd 0.050) | 7.319 s (p95 9.284, sd 0.593) | 4.516 s (p95 6.456, sd 0.589) |
| wide-level | down | 1.757 s (p95 2.022, sd 0.149) | 4.875 s (p95 6.132, sd 0.516) | 2.321 s (p95 2.470, sd 0.079) |
| scale | up | 0.197 s (p95 0.228, sd 0.015) | 1.085 s (p95 1.107, sd 0.023) | 0.611 s (p95 0.634, sd 0.013) |
| scale | down | 269.1 ms (p95 307.3, sd 16.8) | [10 failed of 10] | 440.1 ms (p95 489.3, sd 22.6) |
| network-ipam | up | 110.4 ms (p95 133.0, sd 9.0) | 595.3 ms (p95 690.9, sd 33.4) | 296.4 ms (p95 321.5, sd 10.2) |
| network-ipam | down | 175.9 ms (p95 202.2, sd 13.1) | 509.2 ms (p95 550.5, sd 21.5) | 271.5 ms (p95 299.5, sd 12.4) |
| volume-heavy | up | 105.0 ms (p95 122.8, sd 7.5) | 920.9 ms (p95 944.0, sd 15.0) | 199.1 ms (p95 210.6, sd 8.2) |
| volume-heavy | down | 156.3 ms (p95 165.4, sd 6.0) | 583.3 ms (p95 635.5, sd 21.1) | 249.2 ms (p95 269.4, sd 13.5) |
| secrets | up | 107.3 ms (p95 122.5, sd 5.8) | 471.8 ms (p95 489.3, sd 11.5) | 197.9 ms (p95 208.6, sd 7.2) |
| secrets | down | 159.5 ms (p95 167.3, sd 6.7) | 430.1 ms (p95 449.3, sd 13.4) | 230.8 ms (p95 263.3, sd 12.1) |
| warm-restart | warm up | 42.0 ms (p95 46.8, sd 3.5) | 262.1 ms (p95 273.9, sd 9.4) | 61.8 ms (p95 65.4, sd 3.8) |
| many-services | up | 0.393 s (p95 0.422, sd 0.018) | 2.259 s (p95 2.344, sd 0.039) | 1.372 s (p95 1.768, sd 0.168) |
| many-services | down | 0.496 s (p95 0.620, sd 0.044) | 1.421 s (p95 1.552, sd 0.058) | 0.834 s (p95 0.869, sd 0.033) |
| running-ops | ps | 7.9 ms (p95 9.4, sd 0.8) | 120.9 ms (p95 127.2, sd 2.4) | 48.1 ms (p95 80.8, sd 19.2) |
| running-ops | logs | 8.2 ms (p95 12.2, sd 1.4) | 149.4 ms (p95 151.4, sd 2.7) | 47.5 ms (p95 77.5, sd 15.0) |
| running-ops | exec | 64.2 ms (p95 73.9, sd 5.4) | 202.8 ms (p95 208.5, sd 6.8) | 67.1 ms (p95 108.8, sd 20.4) |
| running-ops | restart | 173.3 ms (p95 191.5, sd 9.5) | 304.4 ms (p95 342.9, sd 20.1) | 267.6 ms (p95 445.4, sd 71.7) |
| wide-running-ops | ps | 11.6 ms (p95 12.7, sd 0.6) | 191.0 ms (p95 196.8, sd 3.8) | 56.1 ms (p95 174.1, sd 50.8) |
| wide-running-ops | logs | 11.3 ms (p95 15.7, sd 1.9) | 206.5 ms (p95 214.5, sd 4.9) | 37.2 ms (p95 86.8, sd 18.9) |
| wide-running-ops | exec | 68.8 ms (p95 74.0, sd 3.8) | 252.0 ms (p95 264.9, sd 5.5) | 50.9 ms (p95 106.5, sd 21.7) |
| wide-running-ops | restart | 123.3 ms (p95 133.3, sd 4.3) | 302.4 ms (p95 311.6, sd 6.7) | 350.9 ms (p95 522.2, sd 112.1) |
| config-heavy | config | 13.3 ms (p95 14.6, sd 0.9) | 554.7 ms (p95 564.2, sd 5.2) | 43.8 ms (p95 48.7, sd 2.8) |
| build | build | 284.9 ms (p95 306.5, sd 8.4) | 368.0 ms (p95 414.5, sd 14.5) | 371.9 ms (p95 398.9, sd 9.5) |

## Memory + CPU per command, each tool on its own engine

Same caveat as the wall-clock table: podup and podman-compose run rootless,
docker-compose on Docker runs rootful. Memory is the orchestrator process;
engine-side work is not charged to any of them.

| scenario | op | podup | podman-compose | docker-compose (Docker) |
|---|---|---|---|---|
| single | up | 6.0 MiB / 7.0 ms | 52.5 MiB / 475.4 ms | 30.0 MiB / 38.8 ms |
| single | down | 6.3 MiB / 8.0 ms | 50.8 MiB / 365.9 ms | 29.4 MiB / 34.7 ms |
| multi-healthcheck | up | 6.0 MiB / 10.2 ms | 52.6 MiB / 736.5 ms | 30.2 MiB / 44.8 ms |
| multi-healthcheck | down | 6.2 MiB / 10.1 ms | 51.1 MiB / 507.4 ms | 29.5 MiB / 36.7 ms |
| deep-chain | up | 6.2 MiB / 0.010 s | 53.0 MiB / 1.363 s | 30.5 MiB / 0.053 s |
| deep-chain | down | 6.3 MiB / 10.9 ms | 51.2 MiB / 874.4 ms | 29.8 MiB / 67.2 ms |
| wide-level | up | 6.9 MiB / 0.031 s | 53.5 MiB / 7.310 s | 34.0 MiB / 0.096 s |
| wide-level | down | 6.7 MiB / 0.026 s | 51.6 MiB / 5.569 s | 31.9 MiB / 0.076 s |
| scale | up | 6.1 MiB / 0.009 s | 52.9 MiB / 1.122 s | 30.2 MiB / 0.042 s |
| scale | down | 6.3 MiB / 9.6 ms | [10 failed of 10] | 30.0 MiB / 37.1 ms |
| network-ipam | up | 6.0 MiB / 7.6 ms | 52.5 MiB / 657.9 ms | 30.2 MiB / 37.9 ms |
| network-ipam | down | 6.2 MiB / 8.7 ms | 50.7 MiB / 495.3 ms | 29.5 MiB / 34.4 ms |
| volume-heavy | up | 6.1 MiB / 0.008 s | 52.4 MiB / 1.110 s | 30.6 MiB / 0.042 s |
| volume-heavy | down | 6.3 MiB / 9.0 ms | 50.5 MiB / 591.7 ms | 30.0 MiB / 37.7 ms |
| secrets | up | 6.0 MiB / 8.6 ms | 52.2 MiB / 499.6 ms | 30.1 MiB / 37.6 ms |
| secrets | down | 6.3 MiB / 8.9 ms | 50.8 MiB / 386.3 ms | 29.4 MiB / 34.9 ms |
| warm-restart | warm up | 6.0 MiB / 7.7 ms | 49.4 MiB / 328.6 ms | 30.2 MiB / 36.8 ms |
| many-services | up | 6.3 MiB / 0.013 s | 53.2 MiB / 2.281 s | 31.2 MiB / 0.055 s |
| many-services | down | 6.4 MiB / 0.013 s | 51.3 MiB / 1.696 s | 30.4 MiB / 0.046 s |
| running-ops | ps | 5.5 MiB / 4.4 ms | 49.2 MiB / 137.5 ms | 29.7 MiB / 46.5 ms |
| running-ops | logs | 5.8 MiB / 4.6 ms | 68.6 MiB / 143.3 ms | 29.8 MiB / 46.7 ms |
| running-ops | exec | 5.6 MiB / 5.4 ms | 48.0 MiB / 143.8 ms | 27.4 MiB / 25.5 ms |
| running-ops | restart | 5.8 MiB / 5.3 ms | 48.9 MiB / 184.0 ms | 29.6 MiB / 36.9 ms |
| wide-running-ops | ps | 5.4 MiB / 5.3 ms | 49.9 MiB / 208.0 ms | 30.9 MiB / 46.3 ms |
| wide-running-ops | logs | 5.7 MiB / 5.4 ms | 68.6 MiB / 207.5 ms | 30.1 MiB / 38.6 ms |
| wide-running-ops | exec | 5.6 MiB / 6.3 ms | 48.1 MiB / 207.5 ms | 27.4 MiB / 21.6 ms |
| wide-running-ops | restart | 5.8 MiB / 6.2 ms | 49.2 MiB / 242.8 ms | 30.4 MiB / 43.4 ms |
| config-heavy | config | 6.0 MiB / 13.1 ms | 34.9 MiB / 556.6 ms | 30.9 MiB / 57.1 ms |
| build | build | 6.0 MiB / 6.0 ms | 65.6 MiB / 402.8 ms | 51.1 MiB / 129.3 ms |

## Reading these numbers honestly

On the same engine podup is fastest in **29 of 29 rows**. Six of those wins
clear the bar by less than two of podup's standard deviations and are better
read as "about the same": `deep-chain down` (0.03 sd against podup's sd of
17.4 ms, gap 0.5 ms), `many-services down` (1.62 sd against sd 44.2 ms, gap
71.5 ms), `multi-healthcheck down` (1.17 sd against sd 111.6 ms, gap 130.8 ms),
`network-ipam down` (1.39 sd against sd 13.1 ms, gap 18.1 ms), `single down`
(1.64 sd against sd 8.1 ms, gap 13.4 ms) and `wide-level down` (1.72 sd
against sd 148.7 ms, gap 255.8 ms). The other 23 wins clear two standard
deviations cleanly.

On a second row, `scale down` for podman-compose, the harness refused the cell
after podman-compose 1.6.0 exited 0 from `down -v` but left `app_2` through
`app_5`, the pod and the network of its compose project behind, on all 10
measured iterations (#1947); podup's own number on that row stands on its own.

Against docker-compose on its own Docker engine, podup is fastest in 28 of 29 rows. The one row it
does not take is `wide-running-ops exec`, where docker-compose on Docker beats
podup 50.9 ms to 68.8 ms (gap 17.9 ms against podup's own standard deviation
of 3.8 ms, about 4.7 sd, narrow in absolute terms because the row is small).
On every other row dockerd is slower: `wide-level up` 4.52 s against podup's
1.13 s, `multi-healthcheck up` 1.36 s against 0.40 s, `deep-chain up` 2.57 s
against 0.36 s, `many-services up` 1.37 s against 0.39 s. The earlier read
that dockerd wins the teardown of many containers (`wide-level down`,
`many-services down`) does not hold here: podup is 1.76 s against 2.32 s on
`wide-level down` and 0.50 s against 0.83 s on `many-services down`. The
engine wins where the row is small and Podman's per-call overhead shows,
and loses where many containers have to be brought up or down.

**Memory.** podup's peak per command is a median of 6.0 MiB across the 29 rows
(worst 6.9, `wide-level up`). It is under the 9.0 MiB budget in
`bench/memory-budget-mib`. #1946 did not reproduce when measured again:
`podup --version` peaks at 3.3 to 3.7 MB on 5.7.1, 5.10.5 and 5.10.7 alike.

podman-compose `config-heavy config` is 554.7 ms here with 1.6.0, on a row that
does not touch the engine at all, against 113 ms with 1.5.0 in the previous
run; the regression persists.

`multi-healthcheck up` still measures healthcheck interval granularity more than
tool speed, and the `secrets` rows still carry the three API calls per secret at
`up` and two at `down` that native Podman secrets cost since 3.1.0.
