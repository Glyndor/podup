#!/usr/bin/env bash
#
# podup installer.
#
# Default: download a release binary, verify it and install it to a directory.
# Updates come from `podup update`.
#
#   curl -fsSL https://glyndor.net/install/podup | bash
#
# --apt (Debian/Ubuntu, amd64): register the podup apt repository and install
# with apt. Updates — including signing-key renewals — come from `apt upgrade`.
#
#   curl -fsSL https://glyndor.net/install/podup | bash -s -- --apt
#
# Environment:
#   PODUP_VERSION      Release tag to install (e.g. v0.3.0). Default: latest.
#   PODUP_INSTALL_DIR  Installation directory (binary mode). Default: /usr/local/bin.

set -euo pipefail

REPO="Glyndor/podup"
INSTALL_DIR="${PODUP_INSTALL_DIR:-/usr/local/bin}"
VERSION="${PODUP_VERSION:-latest}"
MODE="binary"

log_info()  { printf '\033[1;34m[info]\033[0m %s\n' "$1"; }
log_ok()    { printf '\033[1;32m[ ok ]\033[0m %s\n' "$1"; }
log_error() { printf '\033[1;31m[fail]\033[0m %s\n' "$1" >&2; }

fail() {
	log_error "$1"
	exit 1
}

usage() {
	sed -n '3,17p' "$0" | sed 's/^# \{0,1\}//'
	exit 0
}

# --- Arguments ---------------------------------------------------------------

for arg in "$@"; do
	case "$arg" in
		--apt)      MODE="apt" ;;
		-h|--help)  usage ;;
		*)          fail "Unknown argument: ${arg} (try --help)" ;;
	esac
done

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

# --- Shared download / verification ------------------------------------------

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

download() {
	# download <url> <dest>
	curl --proto '=https' --tlsv1.2 -fsSL -o "$2" "$1" || fail "Download failed: $1"
}

# Baked-in base64 (unpadded) raw Ed25519 public keys (32 bytes each) matching the
# release signing key (RELEASE_SIGN_KEY). Up to two are accepted: the second is
# empty except during a key rotation, when it holds the new key so a release
# signed by either key verifies. The signature passes if any key validates.
# Override for a fork via the PODUP_RELEASE_PUBKEY_B64 / _PUBKEY2_B64 env vars.
PODUP_RELEASE_PUBKEY_B64="${PODUP_RELEASE_PUBKEY_B64:-APh+kh61dJeT0HzG+KQXELzDjK4ccvqY9K+FptOZ3+Y}"
PODUP_RELEASE_PUBKEY2_B64="${PODUP_RELEASE_PUBKEY2_B64:-}"

PUBKEYS=()
[[ -n "$PODUP_RELEASE_PUBKEY_B64" ]]  && PUBKEYS+=("$PODUP_RELEASE_PUBKEY_B64")
[[ -n "$PODUP_RELEASE_PUBKEY2_B64" ]] && PUBKEYS+=("$PODUP_RELEASE_PUBKEY2_B64")

# Verify the Ed25519 signature over SHA256SUMS.
#   ed25519_verify <sig-file> <data-file>
# Exit: 0 verified, 1 signature present but INVALID (tampered), 2 cannot verify.
ed25519_verify() {
	[[ ${#PUBKEYS[@]} -gt 0 ]] || return 2
	command -v python3 >/dev/null 2>&1 || return 2
	python3 -c "from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey" 2>/dev/null || return 2
	if python3 - "$1" "$2" "${PUBKEYS[@]}" <<'PYEOF'
import base64, sys
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
from cryptography.exceptions import InvalidSignature
sig_file, data_file = sys.argv[1], sys.argv[2]
sig = open(sig_file, "rb").read()
data = open(data_file, "rb").read()
for pubkey_b64 in sys.argv[3:]:
    try:
        Ed25519PublicKey.from_public_bytes(base64.b64decode(pubkey_b64 + "==")).verify(sig, data)
        sys.exit(0)
    except (InvalidSignature, ValueError):
        continue
sys.exit(1)
PYEOF
	then
		return 0
	fi
	return 1
}

# Confirm a downloaded file matches its entry in SHA256SUMS.
#   verify_checksum <name>
verify_checksum() {
	log_info "Verifying SHA-256 checksum ..."
	(cd "$TMP_DIR" && grep " $1\$" SHA256SUMS | "${SHA256_CMD[@]}" -c --quiet -) \
		|| fail "Checksum verification failed for $1"
	log_ok "Checksum verified"
}

run_root() {
	if [[ "$(id -u)" -eq 0 ]]; then
		"$@"
	elif command -v sudo >/dev/null 2>&1; then
		sudo "$@"
	else
		fail "root privileges required and sudo is not available for: $*"
	fi
}

# --- apt mode ----------------------------------------------------------------

install_apt() {
	[[ "$OS" == "linux" ]] || fail "--apt is only supported on Debian/Ubuntu Linux"
	command -v apt-get >/dev/null 2>&1 || fail "--apt requires apt-get (Debian/Ubuntu)"
	command -v dpkg >/dev/null 2>&1 || fail "--apt requires dpkg"
	[[ "$ARCH" == "x86_64" ]] || fail \
		"the podup apt repository is amd64-only for now; on ${ARCH} install without --apt"

	local kr="podup-archive-keyring.deb"
	log_info "Downloading ${kr} (${VERSION}) ..."
	download "${BASE_URL}/${kr}" "${TMP_DIR}/${kr}"
	download "${BASE_URL}/SHA256SUMS" "${TMP_DIR}/SHA256SUMS"
	download "${BASE_URL}/SHA256SUMS.sig" "${TMP_DIR}/SHA256SUMS.sig"

	# The keyring package is the trust root for every later apt update, so it is
	# verified against the Ed25519 release key before being installed. Build
	# attestation does not cover .deb assets, so the signature is the only anchor.
	local verified=0 rc=0
	log_info "Verifying SHA256SUMS signature ..."
	ed25519_verify "${TMP_DIR}/SHA256SUMS.sig" "${TMP_DIR}/SHA256SUMS" || rc=$?
	case "$rc" in
		0) log_ok "SHA256SUMS signature verified"; verified=1 ;;
		1) fail "SHA256SUMS signature verification failed — release may be tampered" ;;
		2)
			if [[ ${#PUBKEYS[@]} -eq 0 ]]; then
				log_info "no release public key configured — cannot check Ed25519 signature"
			else
				log_info "python3+cryptography not available — cannot check Ed25519 signature"
			fi
			;;
	esac

	if [[ "$verified" -ne 1 ]]; then
		if [[ "${PODUP_INSECURE_SKIP_VERIFY:-0}" == "1" ]]; then
			log_info "PODUP_INSECURE_SKIP_VERIFY=1 — proceeding with checksum verification only"
		else
			fail "Cannot verify the keyring signature. Install python3 with the \
'cryptography' package, set PODUP_RELEASE_PUBKEY_B64, or re-run with \
PODUP_INSECURE_SKIP_VERIFY=1 to accept checksum-only verification."
		fi
	fi

	verify_checksum "$kr"

	log_info "Registering the podup apt repository ..."
	run_root dpkg -i "${TMP_DIR}/${kr}"
	run_root apt-get update
	log_info "Installing podup ..."
	run_root apt-get install -y podup

	log_ok "podup installed via apt: $(podup --version)"
	log_info "Future updates: sudo apt upgrade"
}

# --- binary mode -------------------------------------------------------------

install_binary() {
	log_info "Downloading ${ARTIFACT} (${VERSION}) ..."
	download "${BASE_URL}/${ARTIFACT}" "${TMP_DIR}/${ARTIFACT}"
	download "${BASE_URL}/SHA256SUMS" "${TMP_DIR}/SHA256SUMS"
	download "${BASE_URL}/SHA256SUMS.sig" "${TMP_DIR}/SHA256SUMS.sig"

	# Checksum alone is not a trust anchor: a tampered release can ship a matching
	# SHA256SUMS. The binary is trusted only after at least one cryptographic proof
	# tied to the release key or the repository's build identity succeeds — the
	# Ed25519 signature over SHA256SUMS, or the GitHub build-provenance attestation.
	# If neither verifier can run, the install fails closed. Set
	# PODUP_INSECURE_SKIP_VERIFY=1 to explicitly opt out (checksum only).
	local verified=0 rc=0

	log_info "Verifying SHA256SUMS signature ..."
	ed25519_verify "${TMP_DIR}/SHA256SUMS.sig" "${TMP_DIR}/SHA256SUMS" || rc=$?
	case "$rc" in
		0) log_ok "SHA256SUMS signature verified"; verified=1 ;;
		1) fail "SHA256SUMS signature verification failed — release may be tampered" ;;
		2)
			if [[ ${#PUBKEYS[@]} -eq 0 ]]; then
				log_info "no release public key configured — skipping Ed25519 signature check"
			else
				log_info "python3+cryptography not available — cannot check Ed25519 signature"
			fi
			;;
	esac

	# Build-provenance attestation: proves the binary was produced by this repo's
	# release workflow (GitHub OIDC). Strong even without the release public key.
	if command -v gh >/dev/null 2>&1 && gh attestation --help >/dev/null 2>&1; then
		log_info "Verifying artifact attestation ..."
		gh attestation verify "${TMP_DIR}/${ARTIFACT}" --repo "$REPO" >/dev/null \
			|| fail "Attestation verification failed for ${ARTIFACT}"
		log_ok "Attestation verified"
		verified=1
	else
		log_info "GitHub CLI with attestation support not found — cannot check attestation"
	fi

	# Fail closed unless a strong proof succeeded or the user explicitly opts out.
	if [[ "$verified" -ne 1 ]]; then
		if [[ "${PODUP_INSECURE_SKIP_VERIFY:-0}" == "1" ]]; then
			log_info "PODUP_INSECURE_SKIP_VERIFY=1 — proceeding with checksum verification only"
		else
			fail "No signature or attestation verifier available. Install 'gh' (>= 2.49) \
or python3 with the 'cryptography' package, set PODUP_RELEASE_PUBKEY_B64, or re-run \
with PODUP_INSECURE_SKIP_VERIFY=1 to accept checksum-only verification."
		fi
	fi

	verify_checksum "$ARTIFACT"

	local install_cmd=(install -m 0755 "${TMP_DIR}/${ARTIFACT}" "${INSTALL_DIR}/podup")
	if [[ -w "$INSTALL_DIR" ]]; then
		"${install_cmd[@]}"
	elif command -v sudo >/dev/null 2>&1; then
		log_info "Installing to ${INSTALL_DIR} (requires sudo) ..."
		sudo "${install_cmd[@]}"
	else
		fail "Cannot write to ${INSTALL_DIR} and sudo is not available. Set PODUP_INSTALL_DIR to a writable directory."
	fi

	log_ok "podup installed: $("${INSTALL_DIR}/podup" --version)"
}

# --- Dispatch ----------------------------------------------------------------

if [[ "$MODE" == "apt" ]]; then
	install_apt
else
	install_binary
fi
