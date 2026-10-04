# Self-update security model

`podup update` replaces the running binary with the latest release. The default run resolves the newest release, verifies it, and installs it; `--check` reports whether a newer release exists and then stops. `--force` permits reinstalling a non-newer release. Direct-binary builds provide `podup update`; the Debian package excludes it.

The trust chain is built around an Ed25519 signature over `SHA256SUMS` checked against keys compiled into the binary itself, plus a SHA-256 digest match of the chosen binary against that manifest. The baked-in key is the trust anchor: a release that fails either check cannot be installed, regardless of who controls the network or the download host. Provenance, TLS, and other transport-layer mechanisms are defence in depth, not the gate.

## Trust anchor

The trust anchor is **not** the download domain, DNS, or TLS. It is the set of **Ed25519 public keys compiled into the binary** (`internal/update/verify.rs`, `RELEASE_PUBKEYS`). The matching private key is held only as a CI secret and signs every release in CI (`.github/workflows/release.yml`). Because the public key is baked into a signed binary, an attacker cannot swap the key without invalidating the binary that contains it.

## Verification flow

`podup update` performs, in order, failing closed at the first problem:

1. Resolve GitHub release metadata and compare versions; stop for `--check` or a non-newer release unless forced. On Windows, best-effort stale `.old` cleanup can happen before that return.
2. Refuse a package-manager-managed binary. Before asset downloads, an update-enabled binary refuses replacement if dpkg owns its path, or it resolves inside Homebrew/Scoop layouts; `--force` does not bypass this. Use `apt upgrade podup`, `brew upgrade podup` or `scoop update podup`. Manual/cargo layouts can update. Dpkg refusal may diagnose allowed-origin/blacklist problems, not whether automatic scheduling runs.
3. Verify `SHA256SUMS.sig` against an embedded key before downloading the binary; reject missing/malformed keys or signatures. The manifest and signature are fetched over HTTPS.
4. Look up the binary's expected SHA-256 in the now-trusted `SHA256SUMS`, download the platform binary over HTTPS, and verify its bytes match (the digest comparison runs in constant time).
5. Atomically replace the running executable. The staging file is created with `create_new` (`O_EXCL`) and opened with `O_NOFOLLOW` at mode `0600`; the target's mode is copied while stripping any setuid/setgid/sticky bits, fsync, then `rename` over the target. On Windows the in-use `.exe` is renamed aside first and cleaned up on the next `podup update` run.

Any failure exits with code `3` (distinct from clap's `2` for usage errors and from a generic `1`). Verification failures preserve the binary; post-install version failure attempts restoration, which can fail.

## install.sh

The one-line installer applies the same fail-closed policy. With a release public key configured (the default), the Ed25519 signature check is mandatory: it requires `python3` with the `cryptography` package and refuses to install if the check cannot run. There is no opt-out: a checksum alone is not a trust anchor. The `--apt` path likewise verifies the keyring package's Ed25519 signature before installing it as root.

Binary installers require Python 3 with `cryptography` and verify the signed manifest plus selected digest. If a capable `gh` is installed, installers also verify release-workflow provenance.

### Version self-test (rollback gate)

The signed manifest binds the asset bytes but **not** the release tag. Both `install.sh` and `install.ps1` run the staged binary's `--version` and pin it to the resolved tag (one optional `v` prefix). The comparison is strict equality; mismatch deletes the stage and exits 1. This establishes tag/bytes consistency, not freshness: coherent old metadata/bytes passes. The updater separately rejects non-newer versions unless forced, and tests after replacement with attempted rollback.

```
[fail] Staged binary reports 'podup version v3.6.0', expected v3.7.0
Refusing to install: staged binary's --version does not match the resolved
release tag (possible rollback) - the staged file has been removed
```

Installers require staged `--version` to match the resolved tag. Installer scripts are themselves covered by `SHA256SUMS` and detached signatures.

The embedded Python classifies verification failures into two exit codes:

- **rc=1 (signature mismatch)**: every embedded key rejected the signature. Treat as a release-tampering problem; do not retry.
- **rc=3 (key malformed)**: at least one embedded key was set but could not be decoded into a 32-byte Ed25519 point. Installer decoding tolerates some invalid characters; CI decoding is stricter. Correct key overrides for rc=3; do not bypass signature failures.

## The embedded public keys

Consumers have two key slots: one active and one empty for rotation. Trust succeeds under any accepted nonempty key; no usable keys fails closed. Keys live in `RELEASE_PUBKEYS`, `install.sh`'s two `PODUP_RELEASE_PUBKEY` variables and `install.ps1`'s `PubKeyB64`/`PubKey2B64`. A third slot requires changes to every consumer, not just the CI regex.

### Key rotation

Because each binary accepts up to two keys, the signing key can be rotated **without stranding installed binaries**, provided the outgoing private key is still available to sign the migration release. A two-release transition first ships the new key alongside the old (signed by the old key), so binaries already in the field accept the next release and gain the new key; a later release then retires the old key once every install has converged on the new one. The GitHub build-provenance attestation, which does not depend on the signing key, still proves origin during the window.

Binaries that predate the current key set cannot verify newer releases in-band; they are reinstalled once via the current `install.sh` / apt and update normally from then on.

## Verifying a release independently

Operators who want to verify a release without trusting the installer can do so with standard tooling. Pin the verification to the release workflow so a `--repo`-only check cannot be satisfied by a different workflow with release-write access:

```bash
gh attestation verify podup-linux-x86_64 \
  --repo Glyndor/podup \
  --signer-workflow Glyndor/podup/.github/workflows/release.yml
```

To verify the Ed25519 signature over `SHA256SUMS` offline (no GitHub CLI), use the embedded public key. The public key is embedded in the binary and installers. `python3` with `cryptography` is required; GNU `sha256sum` (or `shasum -a 256 -c` on macOS) is the checksum tool.

Place the selected asset, `SHA256SUMS` and `SHA256SUMS.sig` together. Offline verification needs Python 3 with `cryptography` and GNU `sha256sum` (macOS: `shasum -a 256 -c`). The verification bash block must halt on the first failure:

```bash
(
  set -e
  python3 - <<'PY'
import base64
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
key = Ed25519PublicKey.from_public_bytes(
    base64.b64decode("HFv7vg5FCY7YyKUDbJhaQSfB9SboJGSblJtFbLmLHzM" + "=="))
key.verify(open("SHA256SUMS.sig", "rb").read(), open("SHA256SUMS", "rb").read())
print("SHA256SUMS signature OK")
PY
  test -f podup-linux-x86_64
  sha256sum --check --ignore-missing SHA256SUMS
)
```

`--ignore-missing` skips files not present locally; ensure `podup-linux-x86_64` is in the current directory so the selected digest row is checked.

Each artifact has a signed CycloneDX SBOM; `NOTICES.html` is also signed. Debian package SBOMs currently use GNU targets although the packaged binaries use musl, so their target-specific dependency lists do not match.

## Air-gapped installation

Networks that block outbound GitHub have no opt-out from verification; they carry the artifacts across the boundary instead:

1. On a connected host, download the platform binary, `SHA256SUMS`, `SHA256SUMS.sig` (and optionally the SBOM/NOTICES and their `.sig` files).
2. Transfer them to the isolated host on approved media.
3. Verify the Ed25519 signature and checksum there with the offline snippet above: the embedded key is the trust anchor, so no network is needed.
4. Install the verified binary manually (`install -m 0755 podup-linux-x86_64 /usr/local/bin/podup`). The destination must be writable by the operator running `install`, which means `sudo` or a different path on most systems.

For offline Debian builds, run `cargo vendor vendor` on a connected host and transfer that tree with the sources; `debian/rules` uses frozen/offline Cargo when `vendor/` exists.