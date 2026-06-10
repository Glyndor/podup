# Debian packaging

podup targets inclusion in the official Debian and Ubuntu archives. The
`debian/` directory in this repository builds a working package today; this
page tracks what that gives us and what the official archive still requires.

## Build a .deb locally

```bash
dpkg-buildpackage -us -uc -b
```

Requires `debhelper`, `cargo` and `rustc >= 1.86` (the crate's declared
`rust-version`, driven by the `idna_adapter ≥ 1.2` transitive dependency through
bollard → url → idna). The package installs a single file: `/usr/bin/podup`.

> **Debian compatibility note:** Debian trixie ships rustc 1.85; MSRV 1.86
> requires Debian sid or a backported toolchain. Track
> https://packages.debian.org/rustc for the trixie toolchain version.

## What the skeleton covers

- `debian/control` — source/binary stanzas, build dependencies, `Recommends: podman`
- `debian/rules` — debhelper with cargo overrides, `--locked` release build, tests run during the build
- `debian/copyright` — DEP-5, Apache-2.0
- Source format `3.0 (native)` — the repository is upstream

## Path to the official archive (owner actions)

1. **Fully offline builds** — the archive forbids network access at build
   time. Options: `debcargo` (packages each crate dependency) or a vendored
   source tarball. Decision pending; `debcargo` is the route the Debian Rust
   team maintains.
2. **ITP bug** — file an *Intent to Package* against `wnpp` from the
   maintainer's identity.
3. **Sponsorship** — upload through a Debian Developer (the Rust team's
   `team+rust@tracker.debian.org` is the natural reviewer for a Rust tool).
4. **crates.io publication** — the name `podup` is verified available and
   the crate metadata is in place (`cargo package` verifies clean), so
   publication is a single `cargo publish` with the owner's registry
   token. `debcargo` consumes crates.io releases, so publishing first
   simplifies everything.
5. **Stability promise** — official packages imply SemVer discipline and a
   `1.0.0` once the CLI surface is settled.

## Versioning

`debian/changelog` tracks the upstream version (native package, no `-1`
revision). Bump it as part of each release PR.
