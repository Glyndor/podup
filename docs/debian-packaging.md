# Debian packaging

podup targets inclusion in the official Debian and Ubuntu archives. The
`debian/` directory in this repository builds a working package today; this
page tracks what that gives us and what the official archive still requires.

## Build a .deb locally

```bash
dpkg-buildpackage -us -uc -b
```

Requires `debhelper`, `cargo` and `rustc >= 1.85` (the crate's declared
`rust-version`). The package installs `/usr/bin/podup` and the `podup(1)` man
page.

## Prebuilt .deb from releases

Each tagged release attaches a signed `.deb` per architecture —
`podup_<version>_amd64.deb` and `podup_<version>_arm64.deb` (each with its
`.sig`, and an entry in the release `SHA256SUMS`) built on Debian sid. Install
the one matching your architecture directly:

```bash
sudo apt install ./podup_<version>_amd64.deb   # or _arm64.deb on aarch64
```

`podup update` refuses to self-replace a binary installed this way — it would
desync dpkg's record of the file — and points back to the package manager;
upgrade with `apt` instead.

## apt repository (apt.glyndor.net)

For Debian and Ubuntu (amd64 and arm64), podup is served from the **Glyndor apt
repository** at `https://apt.glyndor.net`, alongside other Glyndor packages.
The repository lives in its own repo, [Glyndor/apt](https://github.com/Glyndor/apt):
a workflow there downloads the latest amd64 `.deb` release asset of each tracked
product, builds a signed `reprepro` repository, and publishes it to GitHub
Pages. It is rebuilt fresh each run, so it always carries the current version of
every package (no old-version support). podup's only responsibility is to attach
a `podup_<version>_<arch>.deb` asset (amd64 and arm64) to each release — which
the `build-deb` matrix in `release.yml` already does.

### One-line setup

```bash
curl -fsSL https://glyndor.net/podup/install/unix | bash -s -- --apt
```

This downloads `glyndor-archive-keyring.deb` over HTTPS, installs it (registering
the signing key and source list), then runs `apt install podup`.

### Manual setup

```bash
curl -fsSLO https://apt.glyndor.net/glyndor-archive-keyring.deb
sudo dpkg -i glyndor-archive-keyring.deb
sudo apt update && sudo apt install podup
```

### Why key renewal is automatic

The signing key ships as the `glyndor-archive-keyring` package, so apt owns the
key file. When the key is rotated or its expiry extended, a new keyring version
is published and `apt upgrade` installs it — nothing for users to re-run.

### Refreshing after a release

The apt repository rebuilds on a daily schedule and can be triggered manually
(`gh workflow run publish.yml -R Glyndor/apt`). For an immediate refresh on each
release, set an `APT_DISPATCH_TOKEN` secret (a token with `contents:write` on
`Glyndor/apt`); `release.yml` then sends a `repository_dispatch` after
publishing. The signing key, keyring builder and repository builder all live in
the `Glyndor/apt` repo.

> **Debian compatibility note:** the MSRV is 1.85, which Debian trixie ships, so
> trixie and sid can both build the package. (Earlier releases needed 1.86 via
> an `idna`/`icu` dependency chain that has since been removed.)

## What the skeleton covers

- `debian/control` — source/binary stanzas, build dependencies, `Recommends: podman`
- `debian/rules` — debhelper with cargo overrides, `--locked` release build, tests run during the build
- `debian/podup.1` + `debian/podup.manpages` — the man page, installed by `dh_installman`
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
5. **Stability promise** — official packages imply SemVer discipline, which
   `1.0.0` (already shipped) puts in force: the CLI surface is stable and
   breaking changes wait for a major bump.

## Versioning

`debian/changelog` tracks the upstream version (native package, no `-1`
revision). Bump it as part of each release PR.
