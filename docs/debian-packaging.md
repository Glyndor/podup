# Debian packaging

Install on Debian/Ubuntu with the [README](../README.md) bootstrap. Packages and archive-key updates arrive through `apt upgrade`. This page covers package dependencies and builds.

## Package dependencies

| Dependency | What it is for |
|---|---|
| `podman (>= 5.0)` | Installed Podman >=5; configure its API socket separately. |
| `unattended-upgrades` | Automatic-update software; scheduling and allowed origins still need enabling. |
| `glyndor-archive-keyring` | Glyndor archive key, source and allowed-origin configuration. |

A fork's keyring must declare `Provides: glyndor-archive-keyring` to satisfy the dependency.

Installed `unattended-upgrades` does not guarantee automatic updates. The schedule is the `APT::Periodic` setting in `/etc/apt/apt.conf.d/20auto-upgrades`, machine-wide policy that neither the package nor the archive bootstrap writes. Without it, podup is upgraded only when you run `apt upgrade`.

## Build a .deb locally

Build prerequisites: `debhelper` 13, `build-essential`, `dpkg-dev`, `musl-tools` and the workflow-pinned rustup toolchain with the matching musl target.

```sh
dpkg-buildpackage -us -uc -b
```

The package builds `watch`/`completions` with `update` compiled out; use `apt` for upgrades.

Installs `/usr/bin/podup`, `podup(1)`, and Bash/Zsh/fish completions. Releases build natively for amd64/arm64 in pinned Debian trixie containers.

## Release artifacts

Releases attach signed `podup_<version>_{amd64,arm64}.deb` assets, also covered by `SHA256SUMS`. Installing them requires the archive-keyring dependency; they are not a separate fresh-host install route.

## What the skeleton covers

- `debian/control`: dependencies.
- `debian/rules`: locked Cargo build/tests through debhelper.
- `debian/podup.1`: installed man page.
- `debian/copyright`: DEP-5 MIT metadata.
- Source format: 3.0 (native).

Official Debian/Ubuntu archive inclusion is not planned. CLI breaking changes require a major release.