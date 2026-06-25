<div align="center">

# podup

**docker-compose on rootless Podman — one static Rust binary. No daemon. No Python.**

[![CI](https://github.com/Glyndor/podup/actions/workflows/ci.yml/badge.svg)](https://github.com/Glyndor/podup/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/podup.svg)](https://crates.io/crates/podup)
[![downloads](https://img.shields.io/crates/d/podup.svg)](https://crates.io/crates/podup)
[![MSRV](https://img.shields.io/badge/MSRV-1.85-orange.svg)](Cargo.toml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

[**Install**](#-install) · [**Quick start**](#-quick-start) · [**Benchmarks**](docs/benchmarks.md) · [**Docs**](docs/)

<img src="docs/assets/podup-demo.gif" alt="podup running a compose stack on rootless Podman" width="760">

</div>

---

## 📥 Install

```bash
curl -fsSL https://glyndor.net/podup/install/unix | bash      # Linux / macOS
```

```powershell
irm https://glyndor.net/podup/install/windows | iex           # Windows
```

Signed, SHA-256 verified, fail-closed. Requires **Podman ≥ 5.0** (rootless).

<details>
<summary><b>apt · build from source · self-update · platforms</b></summary>

### Debian / Ubuntu (apt)

Install from the Glyndor apt repository so updates arrive through `apt upgrade`:

```bash
curl -fsSL https://glyndor.net/podup/install/unix | bash -s -- --apt
```

This installs the `glyndor-archive-keyring` package (registering the signed
repository at `https://apt.glyndor.net`) and then `podup`. Key renewals are
picked up automatically by `apt upgrade`; the apt build omits self-update, since
apt owns upgrades. By hand:

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

`podup update` replaces the running binary in place only after verifying the
release's Ed25519 signature and SHA-256 checksum — it fails closed otherwise. See
[docs/self-update.md](docs/self-update.md) for the trust model.

### Platforms

Linux, macOS and Windows (x86_64 and arm64). On macOS and Windows podup talks to
the `podman machine` VM through its host-side `unix://` socket or `npipe://`
named pipe; the socket must be local (remote `tcp://`/`ssh://` are rejected).

</details>

## 🚀 Quick start

```bash
podup up -d      # start the stack in the current directory
podup ps         # see what's running
podup down -v    # tear down and remove volumes
```

[Every command →](docs/commands.md)

## ⚡ Why

**~7 MiB, no daemon, up to 13× faster than podman-compose.** Rootless-native
libpod API, real compose-spec (`extends`, profiles, `develop.watch`, inline
secrets), and systemd Quadlet export.

[Benchmarks](docs/benchmarks.md) · [vs alternatives](docs/benchmarks.md#vs-alternatives) · [Rust library](https://docs.rs/podup)

## 📖 Docs

[Commands](docs/commands.md) · [Migrating from Compose](docs/docker-migration.md) · [Benchmarks](docs/benchmarks.md) · [Self-update](docs/self-update.md) · [Security model](docs/security-model.md)

## License

[Apache-2.0](LICENSE) — report vulnerabilities privately via the **Security** tab, never in a public issue.
