#!/usr/bin/env bash
#
# Tests for .github/scripts/hygiene/check-secrets.sh. The credential-shaped strings are
# assembled at run time, so this file does not trip the check it tests, and no
# value is ever printed by the script.
set -u

HERE="$(cd "$(dirname "$0")/../../../.." && pwd)"
SCRIPT="$HERE/.github/scripts/hygiene/check-secrets.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
pass=0; fail=0

check() { # <desc> <want> <got>
	if [ "$2" = "$3" ]; then pass=$((pass + 1)); echo "ok    $1"
	else fail=$((fail + 1)); echo "FAIL  $1 (want '$2', got '$3')"; fi
}

rep() { printf '%*s' "$2" '' | tr ' ' "$1"; } # <char> <count>

repo="$WORK/repo"
git init -q "$repo"
gitc() { git -C "$repo" -c user.name=t -c user.email=t@x -c commit.gpgSign=false "$@"; }
echo base > "$repo/README"; gitc add -A; gitc commit -q -m base
base="$(git -C "$repo" rev-parse HEAD)"

# add <file> <line>: commit one new line on top of the base, then scan it.
scan() { # <line>
	git -C "$repo" reset -q --hard "$base"
	printf '%s\n' "$1" > "$repo/probe.txt"; gitc add -A; gitc commit -q -m probe
	(cd "$repo" && "$SCRIPT" "$base" HEAD 2>&1)
}

out="$(scan 'just an ordinary line')"; check "an ordinary line passes" 0 "$?"

flag() { # <description> <line> <pattern name>
	local out rc value
	value="$2"
	out="$(scan "$value")"; rc=$?
	check "$1 fails" 1 "$rc"
	check "$1 is named" 1 "$(printf '%s' "$out" | grep -c "$3")"
	check "$1 reports the file and line, not the value" 1 "$(printf '%s' "$out" | grep -c 'probe.txt:1')"
	check "$1 does not print the value" 0 "$(printf '%s' "$out" | grep -cF -- "$value")"
}

flag "a private key block" "-----BEGIN $(printf 'RSA')$(printf ' PRIVATE KEY')-----" "private key block"
flag "an AWS access key" "id AKI$(printf 'A')$(rep Q 16)" "AWS access key"
flag "a GitHub token" "t=gh$(printf 'p')_$(rep a 36)" "GitHub token"
flag "a fine-grained GitHub token" "github_pat""_$(rep b 60)" "GitHub token"
flag "a JWT" "e""yJ$(rep h 20).e""yJ$(rep p 20).sig" "JWT"
flag "a long value assigned to a secret-like name" "release_secret = \"$(rep c 40)\"" "secret-like name"
flag "a signing key assigned in a config" "SIGNING_KEY: $(rep d 43)" "secret-like name"

# Code that writes or checks the header is not a key; a key inside a string is.
out="$(scan 'let mut pem = String::from("-----BEGIN PRIVATE KEY-----\n");')"
check "code that only writes the header passes" 0 "$?"
out="$(scan '{"private_key":"-----BEGIN PRIVATE KEY-----\nMIIE...fake...\n"}')"
check "a fake key with dots passes" 0 "$?"
flag "a key body inside a string" "{\"k\":\"-----BEGIN PRIVATE KEY-----\\n$(rep M 40)\"}" "private key inside a string"

out="$(scan "api_key = \"$(rep c 40)\" # secret-scan:allow public on purpose")"
check "the allow marker skips the line" 0 "$?"

out="$(scan "token: \${{ secrets.SOMETHING_TOKEN_NAME_THAT_IS_LONG_ENOUGH_TO_MATCH }}")"
check "a reference to a CI secret passes" 0 "$?"

out="$(scan "password_file = \"/run/secrets/$(rep x 40)\"")"
check "a variable that names a file or a path passes" 0 "$?"

out="$(scan "api_key = \"your-$(rep x 40)\"")"
check "a placeholder passes" 0 "$?"

# A secret that was already in the base is not a reason to fail this change.
printf 'old_token = "%s"\n' "$(rep z 40)" > "$repo/old.txt"; gitc add -A; gitc commit -q -m "old"
base2="$(git -C "$repo" rev-parse HEAD)"
echo more >> "$repo/README"; gitc add -A; gitc commit -q -m "touch"
(cd "$repo" && "$SCRIPT" "$base2" HEAD >/dev/null 2>&1); check "an old line in the base is not rescanned" 0 "$?"

# A repository lists the paths that hold throwaway keys on purpose.
git -C "$repo" reset -q --hard "$base"
mkdir -p "$repo/fixtures"
printf -- '-----BEGIN %s-----\n%s\n' 'PRIVATE KEY' "$(rep M 40)" > "$repo/fixtures/test.pem"
printf '# test keys\n^fixtures/\n' > "$repo/.secret-scan-allow"
gitc add -A; gitc commit -q -m "fixture"
(cd "$repo" && "$SCRIPT" "$base" HEAD >/dev/null 2>&1); check "a path listed in .secret-scan-allow is skipped" 0 "$?"
printf -- '-----BEGIN %s-----\n' 'PRIVATE KEY' > "$repo/elsewhere.pem"; gitc add -A; gitc commit -q -m "elsewhere"
(cd "$repo" && "$SCRIPT" "$base" HEAD >/dev/null 2>&1); check "a key outside the listed paths still fails" 1 "$?"

# The header of a key is not always at the start of a line.
H="-----BEGIN $(printf 'PRIVATE')$(printf ' KEY')-----"
body="$(rep M 64)"
scanfile() { # <content> -> output; status in $?
	git -C "$repo" reset -q --hard "$base"
	printf '%s' "$1" > "$repo/probe.yml"; gitc add -A; gitc commit -q -m probe
	(cd "$repo" && "$SCRIPT" "$base" HEAD 2>&1)
}
out="$(scanfile "$(printf 'key: |\n  %s\n  %s\n  %s\n' "$H" "$body" "$body")")"; rc=$?
check "an indented key in YAML fails" 1 "$rc"
check "...and is named" 1 "$(printf '%s' "$out" | grep -c 'private key' | awk '{print ($1 > 0)}')"
check "...without printing the body" 0 "$(printf '%s' "$out" | grep -cF "$body")"
out="$(scanfile "$(printf 'KEY = "%s\n%s\n%s"\n' "$H" "$body" "$body")")"; rc=$?
check "a key in quotes over several lines fails" 1 "$rc"
out="$(scanfile "$(printf 'secrets:\n  - name: k\n    value: %s\n      %s\n' "$H" "$body")")"; rc=$?
check "a header after an assignment, with the body below, fails" 1 "$rc"
out="$(scanfile "x = \"$H\\n\" # a header")"; rc=$?
check "code that writes the header followed by a backslash n passes" 0 "$rc"
out="$(scanfile "$(printf 'HEADER = "%s"\n' "$H")")"; rc=$?
check "a constant that holds the header alone passes" 0 "$rc"
out="$(scanfile "$(printf 'assert!(pem.starts_with("%s"));\n' "$H")")"; rc=$?
check "code that checks the header passes" 0 "$rc"
out="$(scanfile "$(printf 'println!("%s");\nlet next = 1;\n' "$H")")"; rc=$?
check "a header printed by code, followed by code, passes" 0 "$rc"

out="$(scanfile "$(printf 'data = ["%s",\n  "%s",\n]\n' "$H" "$body")")"; rc=$?
check "a header in a list with the body on the next line fails" 1 "$rc"
check "...as a key spread over lines" 1 "$(printf '%s' "$out" | grep -c 'spread over lines')"
out="$(scanfile "$(printf 'key: |\n  %s\n  # the body is below\n' "$H")")"; rc=$?
check "an indented header alone on its line fails" 1 "$rc"
git -C "$repo" reset -q --hard "$base"
printf '*.dat -diff\n' > "$repo/.gitattributes"; printf 'v=%s\n' "gh$(printf 'p')_$(rep a 36)" > "$repo/probe.dat"; gitc add -A; gitc commit -q -m "attributes"
(cd "$repo" && "$SCRIPT" "$base" HEAD >/dev/null 2>&1); check "a file the repository marks as not to be diffed is still read" 1 "$?"

# History: what a commit adds and the next one takes out is still published.
token="gh$(printf 'p')_$(rep a 36)"
git -C "$repo" reset -q --hard "$base"
printf 'ok\n' > "$repo/h.txt"; printf 't=%s\n' "$token" > "$repo/h.txt"; gitc add -A; gitc commit -q -m "add"
printf 'clean\n' > "$repo/h.txt"; gitc add -A; gitc commit -q -m "remove"
out="$(cd "$repo" && "$SCRIPT" "$base" HEAD 2>&1)"; rc=$?
check "a credential added by one commit and removed by the next fails" 1 "$rc"
check "...naming the pattern" 1 "$(printf '%s' "$out" | grep -c 'GitHub token')"
check "...without printing it" 0 "$(printf '%s' "$out" | grep -cF "$token")"
git -C "$repo" reset -q --hard "$base"
printf 'one\n' > "$repo/h.txt"; gitc add -A; gitc commit -q -m "a message that carries $token"
out="$(cd "$repo" && "$SCRIPT" "$base" HEAD 2>&1)"; rc=$?
check "a credential in a commit message fails" 1 "$rc"
check "...reported against the commit, not printed" 0 "$(printf '%s' "$out" | grep -cF "$token")"
git -C "$repo" reset -q --hard "$base"
printf 'one\n' > "$repo/h.txt"; gitc add -A; gitc commit -q -m "plain"
printf 'two\n' > "$repo/h.txt"; gitc add -A; gitc commit -q -m "plain too"
(cd "$repo" && "$SCRIPT" "$base" HEAD >/dev/null 2>&1); check "a range of ordinary commits passes" 0 "$?"
git -C "$repo" reset -q --hard "$base"
printf 'k=%s\n' "$token" > "$repo/g.txt"; gitc add -A; gitc commit -q -m "first adds it"
git -C "$repo" checkout -q -b other "$base"; printf 'x\n' > "$repo/other.txt"; gitc add -A; gitc commit -q -m "other"
git -C "$repo" checkout -q -; git -C "$repo" merge -q --no-ff -m merge other
(cd "$repo" && "$SCRIPT" "$base" HEAD >/dev/null 2>&1); check "a credential in a range that also holds a merge fails" 1 "$?"
git -C "$repo" checkout -q -f "$base" 2>/dev/null; git -C "$repo" branch -q -D other 2>/dev/null

"$SCRIPT" >/dev/null 2>&1; check "no arguments fails" 1 "$?"

echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
