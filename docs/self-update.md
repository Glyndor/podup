# Self-update security model

`podup update` replaces the running binary with the latest release. The design
goal is that **the update source is impossible to tamper with**: no attacker —
even one who controls the network or the download host — can make `podup`
install a modified binary.

## Trust anchor

The trust anchor is **not** the download domain, DNS, or TLS. It is the set of
**Ed25519 public keys compiled into the binary** (`internal/update/verify.rs`,
`RELEASE_PUBKEYS`). The matching private key exists only as the
`RELEASE_SIGN_KEY` GitHub Actions secret and signs every release in CI
(`.github/workflows/release.yml`).

Because the public key is baked into a binary that is itself signed and carries
a GitHub build-provenance attestation, an attacker cannot swap the key without
invalidating the binary that contains it. TLS only protects transport; it is not
relied on for integrity.

## Verification flow

`podup update` performs, in order, failing closed at the first problem:

1. Resolve the latest release tag from the GitHub API and compare it with the
   compiled-in version (`env!("CARGO_PKG_VERSION")`). Nothing is downloaded if
   the current build is already newest (unless `--force`).
2. Download the platform binary, `SHA256SUMS`, and `SHA256SUMS.sig` over HTTPS
   (rustls + webpki roots; HTTPS-only URLs).
3. **Verify the Ed25519 signature** of `SHA256SUMS` against the embedded public
   key. A missing/placeholder key, malformed signature, or bad signature aborts
   here — nothing is written.
4. Look up the binary's expected SHA-256 in the now-trusted `SHA256SUMS` and
   verify the downloaded bytes match.
5. Atomically replace the running executable (write a sibling temp file, fsync,
   preserve mode, then `rename`). On Windows the in-use `.exe` is renamed aside
   first and cleaned up on the next run.

Any failure exits with code `2` and leaves the installed binary untouched.

## install.sh

The one-line installer applies the same fail-closed policy. It requires **at
least one** strong proof to succeed — the Ed25519 signature (when `python3` +
`cryptography` and a configured public key are present) **or** the GitHub
build-provenance attestation (`gh attestation verify`). If neither verifier is
available it refuses to install. There is **no opt-out**: a checksum alone is not
a trust anchor, so a verifier (`gh` or `python3` + `cryptography`) must be present
at install time. The `--apt` path likewise verifies the keyring package's Ed25519
signature before installing it as root.

## The embedded public keys

Each consumer holds **up to two** accepted release keys — an active key plus an
empty rotation slot. The active key (base64
`APh+kh61dJeT0HzG+KQXELzDjK4ccvqY9K+FptOZ3+Y=`) is embedded in three places:

- `internal/update/verify.rs` — `RELEASE_PUBKEYS[0]` (raw 32 bytes); slot 1 is
  `[0u8; 32]` until a second key is rolled in.
- `install.sh` — `PODUP_RELEASE_PUBKEY_B64` default; `PODUP_RELEASE_PUBKEY2_B64`
  is the (empty) rotation slot.
- `install.ps1` — `PubKeyB64` default; `PubKey2B64` is the rotation slot.

A signature is trusted if it validates under **any** non-empty key. The active
key is verified against the genuine published `SHA256SUMS.sig` by the
`embedded_key_verifies_real_release` regression test, so an accidental or
malicious edit to the constant fails CI. If every key is zeroed, both the binary
and the installer fail closed and install nothing. The same key signs `podup`,
`panel`, and `panel-agent` releases (shared `RELEASE_SIGN_KEY`).

### Deriving the public key from the signing secret

To re-derive or rotate it, run locally with the `RELEASE_SIGN_KEY` value (the
raw 32-byte Ed25519 seed, base64). **Never commit or share the private seed** —
only its public half:

```bash
python3 -c '
import base64, sys
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization
seed = base64.b64decode(sys.argv[1] + "==")
pub = Ed25519PrivateKey.from_private_bytes(seed).public_key()
raw = pub.public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
print("install.sh base64 :", base64.b64encode(raw).decode().rstrip("="))
print("verify.rs bytes   :", ", ".join(str(b) for b in raw))
' "$RELEASE_SIGN_KEY"
```

Paste the base64 form into `install.sh` and `install.ps1`, and the byte array
into `RELEASE_PUBKEYS`.

### Key rotation

Because each binary accepts up to two keys, the signing key can be rotated
**without stranding installed binaries** — even if the private key leaks. Run the
two-release procedure:

1. **Transition release.** Add the new key to the rotation slot so the binary
   embeds `[old, new]` (and add `PODUP_RELEASE_PUBKEY2_B64` / `PubKey2B64` to the
   installers). Keep `RELEASE_SIGN_KEY` set to the **old** key so `SHA256SUMS` is
   signed by `old`. Binaries already in the field trust only `old`, so they
   accept this release and upgrade — gaining `new` in the process.
2. **Retire release.** Move `new` into slot 0 and zero the rotation slot so the
   binary embeds `[new]`, and switch the CI `RELEASE_SIGN_KEY` secret to the
   **new** key. Every binary from step 1 trusts `new`, so all installs converge
   on the new key and `old` is retired.

During step 1 the leaked `old` key is still accepted for one release — an
unavoidable cost of any rotation. The GitHub build-provenance attestation, which
does not depend on the signing key, still proves origin during the window.
