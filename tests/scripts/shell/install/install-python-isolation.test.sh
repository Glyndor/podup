#!/usr/bin/env bash
#
# The installer's python must not import from the current directory.
#
# `python3 -c` and `python3 -` put the current directory at the front of
# sys.path. An installer started from a directory another user can write
# to (a download folder, /tmp, somebody else's checkout) then imports
# whatever sits there: a planted `cryptography/` package replaces the real
# one, and a fake `verify()` that always succeeds turns the signature
# check into a no-op the script trusts because the python exited 0. A
# planted `json.py` shadows the standard library module and can name an
# older release as "latest", which is the rollback the version self-test
# refuses to install.
#
# The isolated mode (-I) takes the current directory (and the user site
# directory) out of the search path, so only what the system installed
# can be imported.
#
# The cases plant a module in the current directory and then run the
# real installer functions over it:
#
#   1. ed25519_verify with a planted `cryptography` package: the planted
#      module is never imported, and the check does not report success
#      over a signature it could not verify.
#   2. resolve_release_tag with a planted `json` module and a stubbed
#      curl: the planted module is never imported, and the tag is the
#      one the response body carries.
#   3. Every python invocation in install.sh carries -I.
#
# Requires: bash, python3, sed, grep. Nothing is downloaded and nothing
# is installed.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
INSTALL_SH="$HERE/install.sh"

WORK="$(mktemp -d)"
HELPERS="$WORK/helpers.sh"
PLANTED="$WORK/planted"
STUB="$WORK/stub"
MARKER_CRYPTO="$WORK/marker-cryptography"
MARKER_JSON="$WORK/marker-json"

pass=0
fail=0

check() { # <description> <expected> <actual>
	if [ "$2" = "$3" ]; then
		echo "ok    $1"
		pass=$((pass + 1))
	else
		echo "FAIL  $1"
		echo "        expected: $2"
		echo "        actual:   $3"
		fail=$((fail + 1))
	fi
}

# --- the installer under test ------------------------------------------------
#
# Source the helpers: everything before the dispatch section is function
# definitions and constants, so sourcing it touches neither the network
# nor the system. Same pattern the release fixtures use.
sed '/^# --- Dispatch /,$d' "$INSTALL_SH" > "$HELPERS"
# shellcheck disable=SC1090
source "$HELPERS"
# install.sh installs its own EXIT trap to remove its temporary
# directory; replace it so this test's own cleanup still runs.
trap 'rm -rf "$WORK"' EXIT

# --- the planted modules ----------------------------------------------------
#
# Both write a marker file at import time. The marker is the proof that
# the planted module was loaded; the path is baked into the module so the
# marker cannot go missing because an environment variable did not
# travel.
mkdir -p "$PLANTED/cryptography/hazmat/primitives/asymmetric" \
	"$PLANTED/cryptography/exceptions"

cat > "$PLANTED/cryptography/__init__.py" <<EOF
with open("$MARKER_CRYPTO", "a") as fh:
    fh.write("cryptography imported\n")
EOF

cat > "$PLANTED/cryptography/hazmat/primitives/asymmetric/ed25519.py" <<'EOF'
class Ed25519PublicKey:
    @staticmethod
    def from_public_bytes(raw):
        return Ed25519PublicKey()

    def verify(self, sig, data):
        return None
EOF

cat > "$PLANTED/cryptography/exceptions/__init__.py" <<'EOF'
class InvalidSignature(Exception):
    pass
EOF

cat > "$PLANTED/json.py" <<EOF
with open("$MARKER_JSON", "a") as fh:
    fh.write("json imported\n")


def loads(payload):
    return {"tag_name": "v0.0.0-planted"}
EOF

# The API answer the installer reads. The real network is never touched.
mkdir -p "$STUB"
cat > "$STUB/curl" <<'EOF'
#!/bin/sh
printf '%s\n' '{"tag_name": "v9.9.9-response"}'
EOF
chmod +x "$STUB/curl"

# --- case 1: a planted cryptography package ---------------------------------
#
# The signature is nonsense, so a real verification fails. A planted
# cryptography makes it pass, which is the whole point: the installer
# would then trust a signature it never checked.
printf 'not a signature' > "$WORK/sig"
printf 'not the signed data' > "$WORK/data"
rm -f "$MARKER_CRYPTO"

cd "$PLANTED" || exit 1
rc=0
( ed25519_verify "$WORK/sig" "$WORK/data" ) || rc=$?
cd "$HERE" || exit 1

check 'a planted cryptography package is not imported' "0" \
	"$([ -e "$MARKER_CRYPTO" ] && echo 1 || echo 0)"
check 'the signature check does not report success over a planted module' "1" \
	"$([ "$rc" -ne 0 ] && echo 1 || echo 0)"

# --- case 2: a planted json module -------------------------------------------
#
# The installer resolves "latest" through the releases API and then pins
# the staged binary's --version to the tag it resolved. A planted json
# module names an older release, and the self-test below it then accepts
# that older release as the one being installed.
rm -f "$MARKER_JSON"

cd "$PLANTED" || exit 1
tag="$(PATH="$STUB:$PATH" resolve_release_tag 2>/dev/null || true)"
cd "$HERE" || exit 1

check 'a planted json module is not imported' "0" \
	"$([ -e "$MARKER_JSON" ] && echo 1 || echo 0)"
check 'the resolved tag comes from the response body' "v9.9.9-response" "$tag"

# --- case 3: every python call is isolated -----------------------------------
#
# A reading of the source, so a call added later without -I is caught
# even if no case happens to exercise it. Comment lines are left out:
# they talk about `python3 -c` to explain why it is not used that way.
unguarded="$(grep -nE 'python3[[:space:]]+-' "$INSTALL_SH" \
	| grep -vE '^[0-9]+:[[:space:]]*#' \
	| grep -vE 'python3[[:space:]]+-I[[:space:]]' \
	|| true)"
check 'every python invocation in install.sh runs isolated' "" "$unguarded"

echo
echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
