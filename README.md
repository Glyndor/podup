<div align="center">

# podup

**Your `docker-compose.yml`, on rootless Podman — one static Rust binary. No daemon. No Python.**

[![CI](https://github.com/Glyndor/podup/actions/workflows/ci.yml/badge.svg)](https://github.com/Glyndor/podup/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/podup.svg)](https://crates.io/crates/podup)
[![downloads](https://img.shields.io/crates/d/podup.svg)](https://crates.io/crates/podup)
[![MSRV](https://img.shields.io/badge/MSRV-1.85-orange.svg)](Cargo.toml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

[**Install**](#-install) · [**Quick start**](#-quick-start) · [**Why podup**](#-why-podup) · [**Docs**](docs/)

<img src="docs/assets/podup-demo.gif" alt="podup running a compose stack on rootless Podman" width="760">

</div>

---

## 📥 Install

**Linux / macOS**

```bash
curl -fsSL https://glyndor.net/podup/install/unix | bash
```

**Windows** (PowerShell)

```powershell
irm https://glyndor.net/podup/install/windows | iex
```

Signed binaries, SHA-256 verified, fail-closed. Requires **Podman ≥ 5.0** (rootless).

<details>
<summary><b>apt · build from source · self-update</b></summary>

### Debian / Ubuntu (apt)

Install from the Glyndor apt repository so updates arrive through `apt upgrade`:

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

### Build from source

```bash
cargo build --release
```

### Self-update

```bash
podup update            # download and install the latest signed release
podup update --check    # report whether a newer release exists, install nothing
```

`podup update` replaces the running binary in place, but only after verifying the
release's Ed25519 signature against the public key embedded in your build and
matching its SHA-256 checksum. It fails closed: a bad signature, missing key, or
checksum mismatch aborts before the installed binary is touched. See
[docs/self-update.md](docs/self-update.md) for the trust model.

### Platforms

Linux, macOS and Windows (x86_64 and arm64). On macOS and Windows podup talks to
the `podman machine` VM through its host-side `unix://` socket or `npipe://`
named pipe; the socket must be local (remote `tcp://`/`ssh://` are rejected).

</details>

## 🚀 Quick start

```bash
podup up -d          # start the stack in the current directory
podup ps             # see what's running
podup logs api -f    # follow a service
podup down -v        # tear down and remove volumes
```

Drop-in for `up`/`down`/`ps`/`logs`/`exec`/`run`/`build`/`scale`/`cp`, watch
mode, and Quadlet export — [full command reference](docs/commands.md).

## ⚡ Why podup

- 🦀 **One static binary** — ~7 MiB RAM, near-zero CPU. No daemon, no Python.
- 🔒 **Rootless-native** — drives Podman's libpod REST API directly.
- ⚙️ **Quadlet export** — `generate quadlet` runs your stack under systemd, no daemon.
- 📄 **Real compose-spec** — `extends`, profiles, `develop.watch`, inline secrets.

|  | podup | docker-compose | podman-compose |
|---|---|---|---|
| Runtime | single static binary | Go + Docker daemon | Python + pip |
| Root | not required | usually (daemon) | not required |
| Quadlet export | ✅ | ❌ | ❌ |

⚡ **Faster and lighter on every op measured** — ~7 MiB vs 59–89 MiB, and up to
**13× quicker** than podman-compose. [Full benchmarks & methodology →](docs/benchmarks.md)

## 🦀 Library too

```rust
use podup::{parse_file, podman, Engine};

let file = parse_file("docker-compose.yml".as_ref())?;
let engine = Engine::new(podman::connect(None)?, "myproject".into());
engine.up(&file).await?;
```

`podup = "1"` — stable public API since 1.0 ([SemVer](https://semver.org/), MSRV 1.85).

## 📖 Docs

[Commands](docs/commands.md) · [Migrating from Compose](docs/docker-migration.md) · [Benchmarks](docs/benchmarks.md) · [Self-update](docs/self-update.md) · [Security model](docs/security-model.md)

## Contributing & security

See the org [contributing guide](https://github.com/Glyndor/.github/blob/main/CONTRIBUTING.md).
Report vulnerabilities privately via the **Security** tab, never in a public issue.

## License

[Apache-2.0](LICENSE)
