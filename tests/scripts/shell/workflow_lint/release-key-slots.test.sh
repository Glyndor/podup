#!/usr/bin/env bash
#
# The number of release key slots is the same in the gate and in every consumer
# (.github/scripts/release/check-key-slots.py). A release signed with a slot that
# a consumer does not read passes the gate and is refused by that consumer, and
# releases cannot be taken back, so each place that holds the number is moved on
# its own in a copy of the tree and the check has to see it.
#
# Requires: python3.
# shellcheck disable=SC2016
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
SCRIPT="$HERE/.github/scripts/release/check-key-slots.py"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
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

fresh() {
	rm -rf "$WORK/t"
	mkdir -p "$WORK/t/.github/scripts/release" "$WORK/t/internal/update"
	cp "$HERE/install.sh" "$HERE/install.ps1" "$WORK/t/"
	cp "$HERE/.github/scripts/release/verify-signing-key.py" "$WORK/t/.github/scripts/release/"
	cp "$HERE/internal/update/verify.rs" "$WORK/t/internal/update/"
}
verdict() { python3 -I "$SCRIPT" "$WORK/t" >"$WORK/out" 2>&1; echo $?; }
says() { grep -c -- "$1" "$WORK/out"; }
edit() { # <file in the copy> <python expression over s>
	python3 - "$WORK/t/$1" "$2" <<'PY'
import sys
path, expr = sys.argv[1], sys.argv[2]
s = open(path).read()
t = eval(expr)
assert t != s, "the change did nothing"
open(path, "w").write(t)
PY
}

check "the real tree agrees" 0 "$(python3 -I "$SCRIPT" "$HERE" >/dev/null 2>&1; echo $?)"
fresh
check "a copy of the tree agrees" 0 "$(verdict)"

# The gate takes a third slot and no consumer does.
fresh
edit .github/scripts/release/verify-signing-key.py 's.replace("SLOT_COUNT = 2", "SLOT_COUNT = 3")'
check "a gate that counts three slots against consumers that read two is found" 1 "$(verdict)"
check "...and every count is printed" 6 "$(grep -c '^[0-9]  ' "$WORK/out")"

# A consumer takes a slot the others do not.
fresh
edit install.sh 's.replace("PODUP_RELEASE_PUBKEY2_B64=\"${PODUP_RELEASE_PUBKEY2_B64:-}\"\n", "PODUP_RELEASE_PUBKEY2_B64=\"${PODUP_RELEASE_PUBKEY2_B64:-}\"\nPODUP_RELEASE_PUBKEY3_B64=\"${PODUP_RELEASE_PUBKEY3_B64:-}\"\n")'
check "install.sh that declares a third slot is found" 1 "$(verdict)"
fresh
edit install.sh 's.replace("[[ -n \"$PODUP_RELEASE_PUBKEY2_B64\" ]] && PUBKEYS+=(\"$PODUP_RELEASE_PUBKEY2_B64\")\n", "")'
check "install.sh that declares a slot it never verifies against is found" 1 "$(verdict)"
fresh
edit install.sh 's.replace("PUBKEYS+=(\"$PODUP_RELEASE_PUBKEY2_B64\")\n", "PUBKEYS+=(\"$PODUP_RELEASE_PUBKEY2_B64\")\n[[ -n \"$PODUP_RELEASE_PUBKEY3_B64\" ]] && PUBKEYS+=(\"$PODUP_RELEASE_PUBKEY3_B64\")\n")'
check "install.sh that verifies against a third slot is found" 1 "$(verdict)"
fresh
edit install.ps1 's.replace("\t$PubKeys = @($PubKeyB64, $PubKey2B64 |", "\t$PubKey3B64 = if ($env:PODUP_RELEASE_PUBKEY3_B64) { $env:PODUP_RELEASE_PUBKEY3_B64 } else { \x27\x27 }\n\t$PubKeys = @($PubKeyB64, $PubKey2B64, $PubKey3B64 |")'
check "install.ps1 with a third slot is found" 1 "$(verdict)"
fresh
edit install.ps1 's.replace("@($PubKeyB64, $PubKey2B64 |", "@($PubKeyB64 |")'
check "install.ps1 that leaves a declared slot out of the list is found" 1 "$(verdict)"
fresh
edit internal/update/verify.rs 's.replace("[[u8; 32]; 2]", "[[u8; 32]; 3]")'
check "verify.rs with an array of three is found" 1 "$(verdict)"
fresh
edit internal/update/verify.rs 's.replace("[[u8; 32]; 2]", "[[u8; 32]; 1]")'
check "verify.rs with an array of one is found" 1 "$(verdict)"

# Something that cannot be read is not a pass.
fresh
rm "$WORK/t/install.ps1"
check "a missing installer is not a pass" 2 "$(verdict)"
fresh
edit internal/update/verify.rs 's.replace("RELEASE_PUBKEYS: [[u8; 32]; 2]", "RELEASE_KEYS: [[u8; 32]; 2]")'
check "a constant that cannot be found is not a pass" 2 "$(verdict)"

echo
echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
