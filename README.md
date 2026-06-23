# podup

[![CI](https://github.com/Glyndor/podup/actions/workflows/ci.yml/badge.svg)](https://github.com/Glyndor/podup/actions/workflows/ci.yml)
[![Release](https://github.com/Glyndor/podup/actions/workflows/release.yml/badge.svg)](https://github.com/Glyndor/podup/actions/workflows/release.yml)

**podup** runs your `docker-compose.yml` on rootless Podman — a single static
binary, written in Rust, with no daemon and no Python runtime.

<p align="center">
  <img src="docs/assets/podup-demo.gif" alt="podup up and podup ps running a compose stack on rootless Podman" width="760">
</p>

```mermaid
flowchart LR
	A["docker-compose.yml"] --> B["podup"]
	B -->|"parse · substitute · order"| C["Podman REST API"]
	C --> D["containers"]
	C --> E["networks"]
	C --> F["volumes"]
```

## ✨ Features

- 🚀 **Drop-in workflow** — `up`, `down`, `start`, `stop`, `ps`, `logs`, `exec`, `run`, `cp`, `build`, `pull`, `restart`, `rm`, `kill`, `pause`, `unpause`, `top`, `port`, `images`, `volumes`, `events`, `config` …and more — see the [command reference](docs/commands.md)
- 🔒 **Rootless by design** — drives rootless Podman over its native libpod REST API
- 📄 **Compose-spec parsing** — YAML anchors, `extends`, `include`, profiles, `env_file`, variable substitution with modifiers
- 🔁 **Dependency-aware** — `depends_on` ordering with `service_started`, `service_healthy`, and `service_completed_successfully` conditions
- 🔢 **Replicas** — `scale:`/`deploy.replicas`, the `scale` command, and `up --scale SERVICE=N`, with named replica containers
- 🔐 **Secrets & configs** — inline content, file, environment, and `external: true` Podman-native secret sources, staged securely
- 👀 **Watch mode** — sync, rebuild or restart services on file changes per `develop.watch` rules
- ⚙️ **Systemd Quadlet export** — `generate quadlet` emits native `podman-systemd.unit` files to run your stack under systemd, no daemon
- ⌨️ **Shell completions** — `completions <shell>` for bash, zsh, fish and more (the Debian package installs them)
- 📦 **Single binary** — statically musl-linked on Linux, no runtime dependencies
- 🦀 **Library too** — embed the parser and engine in your own Rust project

## ✅ Requirements

- **Podman ≥ 5.0** — podup talks to Podman's native libpod REST API (the
  `/v5.0.0/libpod` surface) and assumes a Podman 5.x engine. Rootless is the
  default and recommended posture.
- **Supported platforms:**
  - **Linux** (x86_64, arm64) — talks to the rootless Podman socket directly.
  - **macOS** (x86_64, arm64) — via `podman machine` (applehv or vz backend);
    podup uses the host-side `unix://` socket.
  - **Windows** (x86_64, arm64) — via `podman machine`; podup uses the host-side
    `npipe://` named pipe.

The socket must be local: only `unix://` (and `npipe://` on Windows) are
accepted — remote `tcp://`/`ssh://` endpoints are rejected.

## 📥 Install

Linux and macOS:

```bash
curl -fsSL https://glyndor.net/podup/install/unix | bash
```

Windows (PowerShell):

```powershell
irm https://glyndor.net/podup/install/windows | iex
```

Binaries for Linux and macOS (x86_64 and arm64) plus Windows (x86_64 and
arm64), SHA-256 verified, with build provenance attestations. On macOS and
Windows, podup talks to the `podman machine` VM through its host-side socket or
named pipe. Both installers verify the Ed25519 signature over `SHA256SUMS` (or
the GitHub build-provenance attestation) and fail closed otherwise. Or build
from source:

```bash
cargo build --release
```

### Debian / Ubuntu (apt)

On Debian and Ubuntu (amd64 and arm64), install from the Glyndor apt repository
so updates arrive through `apt upgrade`:

```bash
curl -fsSL https://glyndor.net/podup/install/unix | bash -s -- --apt
```

This installs the `glyndor-archive-keyring` package (registering the signed
repository at `https://apt.glyndor.net`) and then `podup`. Because the signing
key ships as a package, key renewals are picked up automatically by `apt
upgrade`; the apt build omits self-update, since apt owns upgrades. To set it up
by hand:

```bash
curl -fsSLO https://apt.glyndor.net/glyndor-archive-keyring.deb
sudo dpkg -i glyndor-archive-keyring.deb
sudo apt update && sudo apt install podup
```

### Updating

```bash
podup update            # download and install the latest signed release
podup update --check    # report whether a newer release exists, install nothing
```

`podup update` replaces the running binary in place, but only after verifying
the release's Ed25519 signature against the public key embedded in your build
and matching its SHA-256 checksum. It fails closed: a bad signature, missing
key, or checksum mismatch aborts before the installed binary is touched. See
[docs/self-update.md](docs/self-update.md) for the trust model. Installing into
a system directory (e.g. `/usr/local/bin`) needs elevation — re-run with `sudo`.

## 🚀 Quick start

```bash
podup up --detach                      # docker-compose.yml in the current directory
podup -f stack.yml -p myapp up -d      # explicit file and project name
podup ps                               # list project containers
podup logs api --follow                # follow one service's logs
podup down --volumes                   # tear down, removing named volumes
podup generate quadlet -o ~/.config/containers/systemd  # emit systemd Quadlet units
```

## ⚖️ vs. alternatives

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

## 📊 Benchmarks

Wall-clock medians, lower is better. **podup** and **podman-compose** both drive
the same Podman, so this is a pure *tool* comparison — identical engine, identical
digest-pinned and pre-pulled images, identical compose file per scenario. Each
number is the median over 10 measured iterations (2 warm-up runs discarded), with
p95 and standard deviation alongside.

| scenario | op | podup | podman-compose |
|---|---|---|---|
| single | up | **0.102** (p95 0.111, sd 0.004) | 0.673 (p95 0.696, sd 0.012) |
| single | down | **0.146** (p95 0.163, sd 0.009) | 0.609 (p95 0.655, sd 0.021) |
| multi-healthcheck | up | **0.272** (p95 1.348, sd 0.531) | 1.056 (p95 1.161, sd 0.040) |
| multi-healthcheck | down | **0.305** (p95 0.352, sd 0.044) | 0.774 (p95 0.832, sd 0.027) |
| scale (×5) | up | **0.431** (p95 0.455, sd 0.017) | 0.706 (p95 0.747, sd 0.021) |
| scale (×5) | down | **0.444** (p95 0.464, sd 0.013) | 0.628 (p95 0.657, sd 0.015) |
| network + IPAM | up | **0.120** (p95 0.133, sd 0.005) | 1.002 (p95 1.036, sd 0.013) |
| network + IPAM | down | **0.218** (p95 0.233, sd 0.008) | 0.763 (p95 0.807, sd 0.030) |
| volume-heavy | up | **0.116** (p95 0.142, sd 0.009) | 1.565 (p95 1.713, sd 0.054) |
| volume-heavy | down | **0.163** (p95 0.175, sd 0.008) | 0.940 (p95 0.993, sd 0.033) |
| warm restart | warm up | **0.027** (p95 0.033, sd 0.004) | 0.660 (p95 0.707, sd 0.023) |
| many-services (12) | up | **0.462** (p95 0.493, sd 0.028) | 3.777 (p95 4.345, sd 0.220) |
| many-services (12) | down | **0.964** (p95 1.010, sd 0.017) | 1.960 (p95 2.211, sd 0.087) |

Host: AMD Ryzen 7 5700X (16 threads), Linux 6.17 x86_64, CPU governor
`performance`, Podman 5.4.2, podman-compose 1.3.0; the tool process pinned with
`taskset`. Measured 2026-06-23. On `multi-healthcheck` the high p95/stdev on
podup's `up` is the `service_healthy` gate — it waits on the dependency's
healthcheck interval, so the tail varies; the median still leads.

> **docker-compose is not in this table.** It drives `dockerd`, a different
> daemon, so including it would be an end-to-end *stack* comparison, not a
> pure-tool one — and this host had no Docker Engine, so it is left out rather
> than estimated.

podup is faster on every scenario measured here, widest on the volume- and
service-heavy stacks. The harness, scenarios, and full methodology live in
[`bench/`](bench/); reproduce with `bench/run.sh`. Every scenario is published,
whoever wins.

## 🦀 Library usage

```rust
use podup::{parse_file, podman, Engine};

#[tokio::main]
async fn main() -> podup::Result<()> {
	let file = parse_file(std::path::Path::new("docker-compose.yml"))?;
	let client = podman::connect(None)?;
	let engine = Engine::new(client, "myproject".to_string());
	engine.up(&file).await?;
	Ok(())
}
```

```toml
[dependencies]
podup = "1"
```

## 🔒 Stability & versioning

podup follows [Semantic Versioning](https://semver.org/). From **1.0.0** onward:

- The CLI surface (subcommands, flags, exit codes) and the library surface re-exported from the crate root (`parse_file`, `collect_diagnostics`, `Engine`, `ComposeError`, …) are covered by the stability guarantee. Breaking changes bump the major version and are called out in the release notes.
- Public enums and the compose/quadlet result structs are `#[non_exhaustive]`, so new variants and fields can be added in a minor release without breaking downstream code — always include a wildcard arm and avoid exhaustive struct construction.
- The libpod wire types are an internal implementation detail (not re-exported) and may change in any release.
- **MSRV: Rust 1.85.** A bump to the minimum supported Rust version is a minor-version change, never a patch.

## 📖 Docs

- [Command reference](docs/commands.md) — every subcommand, its options, and what it does
- [Migrating from Docker Compose](docs/docker-migration.md) — compatibility guide, rootless differences, deprecated fields
- [Self-update](docs/self-update.md) — the `podup update` trust model and verification flow
- [Security model](docs/security-model.md) — privilege posture, trust boundaries, SBOM and air-gap notes
- [Debian packaging](docs/debian-packaging.md) — building and distributing a `.deb`

## Contributing & security

See the org-wide [contributing guide](https://github.com/Glyndor/.github/blob/main/CONTRIBUTING.md).
Report vulnerabilities privately via the Security tab — never in a public issue.

## License

[Apache-2.0](LICENSE)
