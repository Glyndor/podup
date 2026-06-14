# podup

[![CI](https://github.com/Glyndor/podup/actions/workflows/ci.yml/badge.svg)](https://github.com/Glyndor/podup/actions/workflows/ci.yml)
[![Release](https://github.com/Glyndor/podup/actions/workflows/release.yml/badge.svg)](https://github.com/Glyndor/podup/actions/workflows/release.yml)

**podup** runs your `docker-compose.yml` on rootless Podman — a single static
binary, written in Rust, with no daemon and no Python runtime.

```mermaid
flowchart LR
	A["docker-compose.yml"] --> B["podup"]
	B -->|"parse · substitute · order"| C["Podman REST API"]
	C --> D["containers"]
	C --> E["networks"]
	C --> F["volumes"]
```

## ✨ Features

- 🚀 **Drop-in workflow** — `up`, `down`, `start`, `stop`, `ps`, `logs`, `exec`, `run`, `cp`, `build`, `pull`, `restart`, `rm`, `kill`, `pause`, `unpause`, `top`, `port`, `images`, `config`, `watch`
- 🔒 **Rootless by design** — drives rootless Podman over its native libpod REST API
- 📄 **Compose-spec parsing** — YAML anchors, `extends`, `include`, profiles, `env_file`, variable substitution with modifiers
- 🔁 **Dependency-aware** — `depends_on` ordering with `service_started`, `service_healthy`, and `service_completed_successfully` conditions
- 🔢 **Replicas** — `scale:` and `deploy.replicas` with named replica containers
- 🔐 **Secrets & configs** — inline content, file, environment, and `external: true` Podman-native secret sources, staged securely
- 👀 **Watch mode** — sync, rebuild or restart services on file changes per `develop.watch` rules
- 📦 **Single binary** — statically musl-linked on Linux, no runtime dependencies
- 🦀 **Library too** — embed the parser and engine in your own Rust project

## 📥 Install

Linux and macOS:

```bash
curl -fsSL https://glyndor.net/install/podup | bash
```

Windows (PowerShell):

```powershell
irm https://glyndor.net/install/podup.ps1 | iex
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
curl -fsSL https://glyndor.net/install/podup | bash -s -- --apt
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
podup = { git = "https://github.com/Glyndor/podup", tag = "v0.21.0" }
```

## 📖 Docs

- [Command reference](docs/commands.md) — every subcommand, its options, and what it does
- [Migrating from Docker Compose](docs/docker-migration.md) — compatibility guide, rootless differences, deprecated fields
- [Self-update](docs/self-update.md) — the `podup update` trust model and verification flow
- [Debian packaging](docs/debian-packaging.md) — building and distributing a `.deb`

## Contributing & security

See the org-wide [contributing guide](https://github.com/Glyndor/.github/blob/main/CONTRIBUTING.md).
Report vulnerabilities privately via the Security tab — never in a public issue.

## License

[Apache-2.0](LICENSE)
