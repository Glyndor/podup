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

Each tagged release attaches a signed `podup_<version>_amd64.deb` (with its
`.sig`, and an entry in the release `SHA256SUMS`) built on Debian sid. Install
it directly:

```bash
sudo apt install ./podup_<version>_amd64.deb
```

`podup update` refuses to self-replace a binary installed this way — it would
desync dpkg's record of the file — and points back to the package manager;
upgrade with `apt` instead.

## apt repository (apt.glyndor.net)

For amd64 Debian and Ubuntu, a hosted apt repository keeps podup current through
the normal `apt upgrade` flow. The release workflow rebuilds the repository on
every tag — signed with a dedicated Ed25519 OpenPGP key — and publishes it to
GitHub Pages at `https://apt.glyndor.net`. Only the current release is carried;
podup ships no old-version support.

### One-line setup

```bash
curl -fsSL https://glyndor.net/install/podup | bash -s -- --apt
```

This downloads `podup-archive-keyring.deb` from the latest release, verifies it
against the release Ed25519 signature over `SHA256SUMS`, installs it (registering
the key and source list), then runs `apt install podup`.

### Manual setup

```bash
curl -fsSL https://apt.glyndor.net/podup-apt-key.asc \
  | sudo gpg --dearmor -o /usr/share/keyrings/podup.gpg
printf 'Types: deb\nURIs: https://apt.glyndor.net\nSuites: stable\nComponents: main\nArchitectures: amd64\nSigned-By: /usr/share/keyrings/podup.gpg\n' \
  | sudo tee /etc/apt/sources.list.d/podup.sources
sudo apt update && sudo apt install podup
```

### Why key renewal is automatic

The signing key ships as the `podup-archive-keyring` package, so apt owns the
key file. When the key is rotated or its expiry extended, a new keyring version
is published and `apt upgrade` installs it — nothing for users to re-run.
Dropping the key as a plain file instead would make renewals manual, since apt
cannot update a file it does not own.

### Maintainer notes

- Public key committed at `packaging/apt/podup-apt-key.asc`; the private half is
  the org secret `PODUP_APT_GPG_PRIVATE_KEY`, scoped to this repository.
- `packaging/apt/build-keyring.sh` builds the keyring package and
  `packaging/apt/build-repo.sh` builds and signs the `reprepro` repository; the
  latter fails closed if the committed public key does not match the secret.
- amd64-only for now (the `.deb` job is amd64). arm64 users install the
  standalone binary.

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
5. **Stability promise** — official packages imply SemVer discipline and a
   `1.0.0` once the CLI surface is settled.

## Versioning

`debian/changelog` tracks the upstream version (native package, no `-1`
revision). Bump it as part of each release PR.
