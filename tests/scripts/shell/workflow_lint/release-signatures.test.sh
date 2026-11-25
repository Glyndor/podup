#!/usr/bin/env bash
#
# .github/scripts/release/verify-signatures.sh, the check that every file of a release
# carries a signature that verifies against the key consumers embed (the throwaway key in
# a rehearsal). The verifier is a stand-in that records what it is asked, so the cases are
# about what the script asks for and what it does with the answer:
#   - every file the version names, and SHA256SUMS, is verified against its own .sig;
#   - a verifier that refuses one file stops the script;
#   - in a rehearsal the key comes from the throwaway seed and reaches the verifier, and in
#     a real release nothing sets it;
#   - the verifier is started with -I, and the script takes two arguments.
# Each plant breaks one of those in a copy and expects a case to go red.
set -u
export LC_ALL=C

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
REAL="$HERE/.github/scripts/release"
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

# The stand-in interpreter: `public-key-of.py` prints a key, anything else is a verification.
cat >"$WORK/py" <<'PYEOF'
#!/usr/bin/env bash
echo "$*" >>"$LOG"
args=("$@")
[ "${args[0]}" = -I ] || echo "NO-ISOLATION $*" >>"$LOG"
case "$*" in
*public-key-of.py*) echo "THROWAWAYKEY" ;;
*verify-signing-key.py*)
	echo "KEY=${PODUP_VERIFY_ONLY_KEY_B64:-unset}" >>"$LOG"
	if [ -n "$REFUSE" ]; then case "$*" in *"$REFUSE"*) exit 1 ;; esac; fi
	;;
esac
exit 0
PYEOF
chmod +x "$WORK/py"

run() { # <script dir> [VAR=value ...] -> rc; sets LOG
	local dir="$1"
	shift
	LOG="$WORK/log-$RANDOM"
	: >"$LOG"
	mkdir -p "$WORK/cwd"
	(cd "$WORK/cwd" && env LOG="$LOG" REFUSE= "$@" bash "$dir/verify-signatures.sh" "$WORK/py" 1.2.3 >/dev/null 2>&1)
}

cases() { # <script dir>
	local d="$1" rc want
	want=$(($(python3 -I "$d/expected-assets.py" 1.2.3 checksummed | wc -l) + 1))
	run "$d" REHEARSAL=
	rc=$?
	check "a release of a whole set passes" 0 "$rc"
	check "every file and SHA256SUMS is verified" "$want" "$(grep -c 'verify-signing-key.py' "$LOG")"
	check "each file is verified against its own signature" 0 "$(awk '$2 ~ /verify-signing-key.py/ { if ($3 ".sig" != $4) bad++ } END { print bad + 0 }' "$LOG")"
	check "in a release nothing sets the key to verify against" 0 "$(grep -c '^KEY=THROWAWAYKEY' "$LOG")"
	check "in a release the key of the seed is never asked for" 0 "$(grep -c 'public-key-of.py' "$LOG")"
	check "the verifier is started isolated" 0 "$(grep -c '^NO-ISOLATION' "$LOG")"
	run "$d" REHEARSAL=true
	rc=$?
	check "a rehearsal passes" 0 "$rc"
	check "in a rehearsal the key of the throwaway seed is asked for once" 1 "$(grep -c 'public-key-of.py' "$LOG")"
	check "...and reaches every verification" "$want" "$(grep -c '^KEY=THROWAWAYKEY' "$LOG")"
	run "$d" REHEARSAL= REFUSE=SHA256SUMS
	rc=$?
	check "a verifier that refuses one file stops the script" 1 "$([ "$rc" -ne 0 ] && echo 1 || echo 0)"
	(cd "$WORK/cwd" && bash "$d/verify-signatures.sh" >/dev/null 2>&1)
	check "no argument is refused" 2 "$?"
	(cd "$WORK/cwd" && bash "$d/verify-signatures.sh" a b c >/dev/null 2>&1)
	check "too many arguments are refused" 2 "$?"
}

echo "== the script"
cases "$REAL"

echo "== the plants"
plant() { # <description> <sed expression>
	local copy="$WORK/plant-$RANDOM" before="$fail" saved="$pass"
	mkdir -p "$copy"
	cp "$REAL"/* "$copy/"
	sed -E -i "$2" "$copy/verify-signatures.sh"
	if cmp -s "$copy/verify-signatures.sh" "$REAL/verify-signatures.sh"; then
		check "the plant changes the script: $1" changed unchanged
		return
	fi
	cases "$copy" >/dev/null
	local caught=$((fail - before))
	fail="$before"
	pass="$saved"
	check "plant: $1" 1 "$([ "$caught" -gt 0 ] && echo 1 || echo 0)"
}
plant "the key of a rehearsal not exported" 's/^\texport PODUP_VERIFY_ONLY_KEY_B64$/\ttrue/'
plant "a rehearsal verifying against nothing" 's/^if \[ "\$\{REHEARSAL:-\}" = true \]; then$/if false; then/'
# shellcheck disable=SC2016 # the expressions are literal script text
plant "a file verified against another signature" 's#"\$file" "\$file.sig"#"$file" "SHA256SUMS.sig"#'
# shellcheck disable=SC2016 # the expressions are literal script text
plant "the verifier not isolated" 's/"\$venv_py" -I "\$here\/verify-signing-key.py"/"$venv_py" "$here\/verify-signing-key.py"/'
plant "SHA256SUMS left out" 's/^\techo SHA256SUMS$/\ttrue/'
# shellcheck disable=SC2016 # the expressions are literal script text
plant "a refusal ignored" 's#^\t"\$venv_py" -I "\$here/verify-signing-key.py" "\$file" "\$file.sig"$#\t"$venv_py" -I "$here/verify-signing-key.py" "$file" "$file.sig" || true#'

echo
echo "passed: $pass  failed: $fail"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
