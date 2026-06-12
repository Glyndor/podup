# Self-update security model

`podup update` replaces the running binary with the latest release. The design
goal is that **the update source is impossible to tamper with**: no attacker —
even one who controls the network or the download host — can make `podup`
install a modified binary.

## Trust anchor

The trust anchor is **not** the download domain, DNS, or TLS. It is an **Ed25519
public key compiled into the binary** (`internal/update/verify.rs`,
`RELEASE_PUBKEY`). The matching private key exists only as the
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
available it refuses to install. `PODUP_INSECURE_SKIP_VERIFY=1` is the explicit,
documented opt-out (checksum only) for constrained environments.

## The embedded public key

The release public key is embedded in three places, all holding the same key
(base64 `APh+kh61dJeT0HzG+KQXELzDjK4ccvqY9K+FptOZ3+Y=`):

- `internal/update/verify.rs` — `RELEASE_PUBKEY` (raw 32 bytes).
- `install.sh` — `PODUP_RELEASE_PUBKEY_B64` default (base64, unpadded).
- `install.ps1` — `PubKeyB64` default (base64, unpadded).

It is verified against the genuine published `SHA256SUMS.sig` by the
`embedded_key_verifies_real_release` regression test, so an accidental or
malicious edit to the constant fails CI. If the key is ever zeroed, both the
binary and the installer fail closed and install nothing. The same key signs
`podup`, `panel`, and `panel-agent` releases (shared `RELEASE_SIGN_KEY`).

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
into `RELEASE_PUBKEY`.

### Key rotation

Rotating the signing key requires shipping a new binary that embeds the new
public key *before* the first release signed with the new private key — older
binaries verify only against the key they were built with. Plan a release that
updates the embedded key, then switch the CI secret on the following release.
