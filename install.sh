#!/usr/bin/env bash
#
# podup installer — downloads a release binary, verifies it and installs it.
#
# Usage:
#   curl -fsSL https://glyndor.net/install/podup | bash
#
# Environment:
#   PODUP_VERSION      Release tag to install (e.g. v0.3.0). Default: latest.
#   PODUP_INSTALL_DIR  Installation directory. Default: /usr/local/bin.

set -euo pipefail

REPO="Glyndor/podup"
INSTALL_DIR="${PODUP_INSTALL_DIR:-/usr/local/bin}"
VERSION="${PODUP_VERSION:-latest}"

log_info()  { printf '\033[1;34m[info]\033[0m %s\n' "$1"; }
log_ok()    { printf '\033[1;32m[ ok ]\033[0m %s\n' "$1"; }
log_error() { printf '\033[1;31m[fail]\033[0m %s\n' "$1" >&2; }

fail() {
	log_error "$1"
	exit 1
}

# --- Platform detection ------------------------------------------------------

case "$(uname -s)" in
	Linux)  OS="linux" ;;
	Darwin) OS="darwin" ;;
	*)      fail "Unsupported OS: $(uname -s) (supported: Linux, macOS)" ;;
esac

case "$(uname -m)" in
	x86_64)          ARCH="x86_64" ;;
	aarch64 | arm64) ARCH="arm64" ;;
	*)               fail "Unsupported architecture: $(uname -m) (supported: x86_64, arm64)" ;;
esac

ARTIFACT="podup-${OS}-${ARCH}"

# --- Download ----------------------------------------------------------------

command -v curl >/dev/null 2>&1 || fail "curl is required"

# macOS ships shasum instead of sha256sum.
if command -v sha256sum >/dev/null 2>&1; then
	SHA256_CMD=(sha256sum)
elif command -v shasum >/dev/null 2>&1; then
	SHA256_CMD=(shasum -a 256)
else
	fail "sha256sum or shasum is required"
fi

if [[ "$VERSION" == "latest" ]]; then
	BASE_URL="https://github.com/${REPO}/releases/latest/download"
elif [[ "$VERSION" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
	BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"
else
	fail "PODUP_VERSION must be 'latest' or a semver tag like v1.2.3, got: ${VERSION}"
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

log_info "Downloading ${ARTIFACT} (${VERSION}) ..."
curl --proto '=https' --tlsv1.2 -fsSL -o "${TMP_DIR}/${ARTIFACT}" \
	"${BASE_URL}/${ARTIFACT}" || fail "Download failed: ${BASE_URL}/${ARTIFACT}"
curl --proto '=https' --tlsv1.2 -fsSL -o "${TMP_DIR}/SHA256SUMS" \
	"${BASE_URL}/SHA256SUMS" || fail "Download failed: ${BASE_URL}/SHA256SUMS"
curl --proto '=https' --tlsv1.2 -fsSL -o "${TMP_DIR}/SHA256SUMS.sig" \
	"${BASE_URL}/SHA256SUMS.sig" || fail "Download failed: ${BASE_URL}/SHA256SUMS.sig"

# --- Verify ------------------------------------------------------------------

log_info "Verifying SHA256SUMS signature ..."
# Base64-encoded raw Ed25519 public key (32 bytes) matching RELEASE_SIGN_KEY.
# Set PODUP_RELEASE_PUBKEY_B64 to override (e.g. for testing a fork).
PODUP_RELEASE_PUBKEY_B64="${PODUP_RELEASE_PUBKEY_B64:-}"
if [[ -n "$PODUP_RELEASE_PUBKEY_B64" ]]; then
	if command -v python3 >/dev/null 2>&1 && python3 -c "from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey" 2>/dev/null; then
		if python3 - "${TMP_DIR}/SHA256SUMS.sig" "${TMP_DIR}/SHA256SUMS" "$PODUP_RELEASE_PUBKEY_B64" <<'PYEOF'
import base64, sys
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
from cryptography.exceptions import InvalidSignature
sig_file, data_file, pubkey_b64 = sys.argv[1], sys.argv[2], sys.argv[3]
key = Ed25519PublicKey.from_public_bytes(base64.b64decode(pubkey_b64 + "=="))
try:
    key.verify(open(sig_file, "rb").read(), open(data_file, "rb").read())
    sys.exit(0)
except InvalidSignature:
    sys.exit(1)
PYEOF
		then
			log_ok "SHA256SUMS signature verified"
		else
			fail "SHA256SUMS signature verification failed — release may be tampered"
		fi
	else
		log_info "python3+cryptography not available — skipping SHA256SUMS signature verification"
	fi
else
	log_info "PODUP_RELEASE_PUBKEY_B64 not set — skipping SHA256SUMS signature verification"
fi

log_info "Verifying SHA-256 checksum ..."
(cd "$TMP_DIR" && grep " ${ARTIFACT}\$" SHA256SUMS | "${SHA256_CMD[@]}" -c --quiet -) \
	|| fail "Checksum verification failed for ${ARTIFACT}"
log_ok "Checksum verified"

# Verify the build provenance attestation when the GitHub CLI supports it
# (gh >= 2.49). This proves the binary was built by this repository's release
# workflow. When the subcommand actually runs and fails, abort.
if command -v gh >/dev/null 2>&1 && gh attestation --help >/dev/null 2>&1; then
	log_info "Verifying artifact attestation ..."
	gh attestation verify "${TMP_DIR}/${ARTIFACT}" --repo "$REPO" >/dev/null \
		|| fail "Attestation verification failed for ${ARTIFACT}"
	log_ok "Attestation verified"
else
	log_info "GitHub CLI with attestation support not found — skipping attestation verification (checksum already verified)"
fi

# --- Install -----------------------------------------------------------------

INSTALL_CMD=(install -m 0755 "${TMP_DIR}/${ARTIFACT}" "${INSTALL_DIR}/podup")

if [[ -w "$INSTALL_DIR" ]]; then
	"${INSTALL_CMD[@]}"
elif command -v sudo >/dev/null 2>&1; then
	log_info "Installing to ${INSTALL_DIR} (requires sudo) ..."
	sudo "${INSTALL_CMD[@]}"
else
	fail "Cannot write to ${INSTALL_DIR} and sudo is not available. Set PODUP_INSTALL_DIR to a writable directory."
fi

log_ok "podup installed: $("${INSTALL_DIR}/podup" --version)"
