#!/usr/bin/env bash
#
# The apt path must refuse a keyring package that is not the keyring.
#
# The Ed25519 signature is over raw bytes: it says the bytes carry the
# release key's signature and nothing else. One key signs every artifact
# the release publishes - the binaries, the manifest, the SBOM - so any
# .deb published under it passes the signature check. An older keyring,
# or a package that is not a keyring at all, served in the keyring's
# place is installed as root with the repository's trust anchored to it.
#
# The name the package declares is what tells the two apart, so the
# installer reads it with `dpkg-deb -f` after the signature verifies and
# before the package is installed.
#
# The cases drive the real install_apt with the network, the signature
# check and the privileged steps stubbed out:
#
#   1. A package that declares glyndor-archive-keyring is installed.
#   2. A package that declares another name is refused, and `dpkg -i` is
#      never reached.
#   3. A package whose metadata cannot be read is refused the same way.
#
# Nothing is downloaded and nothing is installed: the privileged steps
# are recorded, not run.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
INSTALL_SH="$HERE/install.sh"

WORK="$(mktemp -d)"
HELPERS="$WORK/helpers.sh"
STUB="$WORK/stub"
DEB_PACKAGE_FILE="$WORK/deb-package"

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
# nor the system.
sed '/^# --- Dispatch /,$d' "$INSTALL_SH" > "$HELPERS"
# shellcheck disable=SC1090
source "$HELPERS"
# install.sh installs its own EXIT trap to remove its temporary
# directory; replace it so this test's own cleanup still runs.
trap 'rm -rf "$WORK"' EXIT

# --- stubs -------------------------------------------------------------------
#
# The stubs answer the questions the installer asks and record what it
# asked, so the case can tell which step refused.
mkdir -p "$STUB"

# dpkg-deb answers with the Package field of the package under test. A
# missing file means the metadata cannot be read.
cat > "$STUB/dpkg-deb" <<'EOF'
#!/bin/sh
[ -f "$STUB_DEB_PACKAGE_FILE" ] || exit 1
cat "$STUB_DEB_PACKAGE_FILE"
EOF

# dpkg and apt-get only have to exist: the privileged step is recorded
# by the run_root stub below, never executed.
cat > "$STUB/dpkg" <<'EOF'
#!/bin/sh
exit 0
EOF

cat > "$STUB/apt-get" <<'EOF'
#!/bin/sh
exit 0
EOF

cat > "$STUB/podup" <<'EOF'
#!/bin/sh
echo "podup version 1.0.0"
EOF

chmod +x "$STUB/dpkg-deb" "$STUB/dpkg" "$STUB/apt-get" "$STUB/podup"

# The download is replaced: the bytes are never fetched, and the file
# the installer asked for is what it gets.
download() {
	printf 'not a real package\n' > "$2"
}

# The signature check is replaced: this case is about the package name,
# and the signature is exercised by the release fixture.
ed25519_verify() {
	return 0
}

# The privileged steps are recorded, not run. `dpkg -i` here would
# install a package on the machine running the test.
run_root() {
	printf '%s\n' "$*" >> "$WORK/root.log"
}

export STUB_DEB_PACKAGE_FILE="$DEB_PACKAGE_FILE"
export PATH="$STUB:$PATH"

# --- run one case ------------------------------------------------------------
#
# run_case <package-name>
#
# Writes the package name the stubbed dpkg-deb answers with, runs
# install_apt, and leaves the exit code in $rc, the output in $out and
# the recorded privileged steps in $root_log.
run_case() {
	printf '%s\n' "$1" > "$DEB_PACKAGE_FILE"
	rm -f "$WORK/root.log"
	set +e
	( install_apt ) > "$WORK/out" 2>&1
	rc=$?
	set -e
	out="$(cat "$WORK/out")"
	root_log=""
	if [ -f "$WORK/root.log" ]; then
		root_log="$(cat "$WORK/root.log")"
	fi
}

# --- case 1: the package is the keyring --------------------------------------
run_case "glyndor-archive-keyring"

check 'the keyring package is installed' "0" "$rc"
check 'and dpkg is asked to install it' "1" \
	"$(printf '%s\n' "$root_log" | grep -c '^dpkg -i ' || true)"
check 'and the install goes on to apt' "1" \
	"$(printf '%s\n' "$root_log" | grep -c '^apt-get install -y podup$' || true)"

# --- case 2: the package declares another name ------------------------------
run_case "podup"

check 'a package that is not the keyring is refused' "1" \
	"$([ "$rc" -ne 0 ] && echo 1 || echo 0)"
check 'and it is never handed to dpkg' "0" \
	"$(printf '%s\n' "$root_log" | grep -c '^dpkg -i ' || true)"
check 'and the refusal names the package it found' "1" \
	"$(printf '%s' "$out" | grep -qF "its Package field is 'podup'" && echo 1 || echo 0)"
check 'and the refusal names the package it wanted' "1" \
	"$(printf '%s' "$out" | grep -q 'glyndor-archive-keyring' && echo 1 || echo 0)"

# --- case 3: the metadata cannot be read -------------------------------------
rm -f "$DEB_PACKAGE_FILE"
rm -f "$WORK/root.log"
set +e
( install_apt ) > "$WORK/out" 2>&1
rc=$?
set -e
out="$(cat "$WORK/out")"
root_log=""
if [ -f "$WORK/root.log" ]; then
	root_log="$(cat "$WORK/root.log")"
fi

check 'a package whose metadata cannot be read is refused' "1" \
	"$([ "$rc" -ne 0 ] && echo 1 || echo 0)"
check 'and it is never handed to dpkg either' "0" \
	"$(printf '%s\n' "$root_log" | grep -c '^dpkg -i ' || true)"

echo
echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
