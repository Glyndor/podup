#!/usr/bin/env bash
#
# Tests for the exemption of one line of one file in .secret-scan-allow (`path :: text`): the line
# is skipped and nothing else is, the list counts once it is in the base, and an entry for a
# line that is gone fails. The credential-shaped strings are assembled at run time, so this
# file does not trip the check it tests, and no value is ever printed by the script.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
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
v="$(rep k 40)"
# the one line that the production file really has, and its false positive
fp="  secret: \"CONFIG_SIGNING_SECRET_KEY_""NAME_FIELD_LONG\","
other="  OTHER_SECRET: \"$v\","

reset_to() { git -C "$repo" reset -q --hard "$1"; git -C "$repo" clean -qfd; mkdir -p "$repo/config" "$repo/src"; }
commit_all() { gitc add -A; gitc commit -q -m "$1"; }
scan() { (cd "$repo" && "$SCRIPT" "$base" HEAD 2>&1); }

echo base > "$repo/README"; commit_all base
bare="$(git -C "$repo" rev-parse HEAD)"

# base: the file as the repository has it (with the line once), and the list with an entry for
# that line and one for a line that is not there yet
mkdir -p "$repo/config"
printf 'export const c = {\n%s\n};\n' "$fp" > "$repo/config/index.ts"
printf '# the name of a variable, not a value\nconfig/index.ts :: %s\nconfig/later.ts :: %s\n' "$fp" "$other" > "$repo/.secret-scan-allow"
commit_all "the file and the list"
base="$(git -C "$repo" rev-parse HEAD)"
add_line() { printf '%s\n' "$1" >> "$repo/config/index.ts"; commit_all "adds a line"; } # <text>

# 1. without the entry the line is a finding, with it it is not
git -C "$repo" checkout -q -b nolist "$bare"
mkdir -p "$repo/config"; printf 'export const c = {\n%s\n};\n' "$fp" > "$repo/config/index.ts"; commit_all "the file"
out="$(cd "$repo" && "$SCRIPT" "$bare" HEAD 2>&1)"; rc=$?
check "the line is a finding when no list exempts it" "1 1" "$rc $(printf '%s' "$out" | grep -c 'config/index.ts:2 long value assigned')"
git -C "$repo" checkout -q -f - ; git -C "$repo" branch -q -D nolist
reset_to "$base"
add_line "$fp"
out="$(scan)"; rc=$?
check "an exempted line that a change adds passes" "0 1" "$rc $(printf '%s' "$out" | grep -c 'no credential-shaped lines added')"

# 2. only that line: a real credential on another line of the same file is still found
reset_to "$base"
printf '%s\n%s\n' "$fp" "$other" >> "$repo/config/index.ts"; commit_all "two lines"
out="$(scan)"; rc=$?
check "another line of the same file is still read" "1 1" "$rc $(printf '%s' "$out" | grep -c 'config/index.ts:5 long value assigned')"
check "and the exempted one is not named" 0 "$(printf '%s' "$out" | grep -c 'config/index.ts:4 ')"
check "and no value is printed" 0 "$(printf '%s' "$out" | grep -cF "$v")"

# 3. the same text in another file is not exempt
reset_to "$base"
printf '%s\n' "$fp" > "$repo/src/other.ts"; commit_all "another file"
out="$(scan)"; rc=$?
check "the same line in another file is a finding" "1 1" "$rc $(printf '%s' "$out" | grep -c 'src/other.ts:1 long value assigned')"

# 4. blanks around the line do not matter, a different line does
reset_to "$base"
printf '\t%s   \n' "${fp#"${fp%%[![:space:]]*}"}" >> "$repo/config/index.ts"; commit_all "blanks"
(cd "$repo" && "$SCRIPT" "$base" HEAD >/dev/null 2>&1); check "other blanks around the same text still pass" 0 "$?"
reset_to "$base"
add_line "${fp}x"
(cd "$repo" && "$SCRIPT" "$base" HEAD >/dev/null 2>&1); check "a line that only starts with the text is not exempt" 1 "$?"
reset_to "$base"
add_line "${fp%?}"
(cd "$repo" && "$SCRIPT" "$base" HEAD >/dev/null 2>&1); check "a line that is only part of the text is not exempt" 1 "$?"
reset_to "$base"
add_line "x = 1 // ${fp#"${fp%%[![:space:]]*}"}"
(cd "$repo" && "$SCRIPT" "$base" HEAD >/dev/null 2>&1); check "the text inside a longer line is not exempt" 1 "$?"

# 5. the list counts once it is in the base: an entry that the change brings exempts nothing
reset_to "$bare"
mkdir -p "$repo/config"
printf 'config/index.ts :: %s\n' "$fp" > "$repo/.secret-scan-allow"
printf '%s\n' "$fp" > "$repo/config/index.ts"; commit_all "the list and the line together"
(cd "$repo" && "$SCRIPT" "$bare" HEAD >/dev/null 2>&1); check "a line entry added by the change itself exempts nothing" 1 "$?"
# an entry the change adds is not in effect yet and is not asked for a line that may not exist yet
reset_to "$base"
printf '%s\nconfig/new.ts :: a line that is not there yet\n' "$(cat "$repo/.secret-scan-allow")" > "$repo/.secret-scan-allow"
commit_all "a third entry"
(cd "$repo" && "$SCRIPT" "$base" HEAD >/dev/null 2>&1); check "an entry the change adds is not asked for its line" 0 "$?"
# an entry whose line the base file never had is waiting for it, and fails nothing
reset_to "$base"
echo more >> "$repo/README"; commit_all "unrelated"
(cd "$repo" && "$SCRIPT" "$base" HEAD >/dev/null 2>&1); check "an entry for a line that has not come yet fails nothing" 0 "$?"
reset_to "$base"
printf '%s\n' "$other" > "$repo/config/later.ts"; commit_all "the line comes"
out="$(scan)"; rc=$?
check "and when the line comes it is exempt" "0 1" "$rc $(printf '%s' "$out" | grep -c 'no credential-shaped lines added')"
reset_to "$base"
printf '%s\n' "$fp" > "$repo/config/later.ts"; printf '  OTHER_SECRET: "%s",\n' "$(rep m 40)" >> "$repo/config/later.ts"; commit_all "another line of that file"
(cd "$repo" && "$SCRIPT" "$base" HEAD >/dev/null 2>&1); check "while another line of that file is read" 1 "$?"

# an entry the change adds for a line that it adds does not exempt it, with a list in the base
reset_to "$base"
printf '%s\nconfig/index.ts :: %s\n' "$(cat "$repo/.secret-scan-allow")" "$other" > "$repo/.secret-scan-allow"
printf '%s\n' "$other" >> "$repo/config/index.ts"; commit_all "the entry and its line together"
out="$(cd "$repo" && "$SCRIPT" "$base" HEAD 2>&1)"; rc=$?
check "a line entry the change adds does not exempt the line the change adds" "1 1" "$rc $(printf '%s' "$out" | grep -c 'config/index.ts:4 long value assigned')"
check "and the entry itself, for a line the file has, is not what is named" 0 "$(printf '%s' "$out" | grep -c '^::error::.secret-scan-allow:')"
# an entry for a line that the file already has is how a false positive is exempted from then on
reset_to "$base"
printf '%s\n' "$other" > "$repo/config/old.ts"; commit_all "a file that already holds the line"
base2="$(git -C "$repo" rev-parse HEAD)"
printf '%s\nconfig/old.ts :: %s\n' "$(cat "$repo/.secret-scan-allow")" "$other" > "$repo/.secret-scan-allow"; commit_all "its entry"
out="$(cd "$repo" && "$SCRIPT" "$base2" HEAD 2>&1)"; rc=$?
check "an entry for a line that the file has passes, though its text is shaped like a credential" "0 1" "$rc $(printf '%s' "$out" | grep -c 'no credential-shaped lines added')"
# an entry for a line the file does not have yet is read like any other line of the repository
reset_to "$base"
printf '%s\nconfig/new.ts :: %s\n' "$(cat "$repo/.secret-scan-allow")" "$other" > "$repo/.secret-scan-allow"; commit_all "an entry for a line to come"
out="$(cd "$repo" && "$SCRIPT" "$base" HEAD 2>&1)"; rc=$?
check "an entry for a line that is not there yet is flagged when its text looks like a credential" "1 1" "$rc $(printf '%s' "$out" | grep -c '.secret-scan-allow:')"

# 6. dead entries: the base file had the line and the change takes it away
reset_to "$base"
printf 'export const c = {};\n' > "$repo/config/index.ts"; commit_all "the line is gone"
out="$(scan)"; rc=$?
check "an entry whose line is not in the file any more fails" "1 1" "$rc $(printf '%s' "$out" | grep -c 'that is not in the file any more')"
check "and the message names the file and not the text" "1 0" "$(printf '%s' "$out" | grep -c 'config/index.ts') $(printf '%s' "$out" | grep -cF 'CONFIG_SIGNING')"
reset_to "$base"
printf 'one\n' > "$repo/config/index.ts"; commit_all "the file has other lines"
(cd "$repo" && "$SCRIPT" "$base" HEAD >/dev/null 2>&1); check "it fails even if nothing was added that looks like a credential" 1 "$?"
reset_to "$base"
printf 'export const c = {};\n' > "$repo/config/index.ts"; printf '# nothing is exempt now\n' > "$repo/.secret-scan-allow"; commit_all "the line and the entries go together"
(cd "$repo" && "$SCRIPT" "$base" HEAD >/dev/null 2>&1); check "removing the line and its entry in one change passes" 0 "$?"
reset_to "$base"
git -C "$repo" rm -q -f config/index.ts; commit_all "the file is deleted"
(cd "$repo" && "$SCRIPT" "$base" HEAD >/dev/null 2>&1); check "an entry for a file that is gone fails" 1 "$?"
reset_to "$base"
printf 'export const d = 1;\n' > "$repo/config/other.ts"; commit_all "unrelated"
(cd "$repo" && "$SCRIPT" "$base" HEAD >/dev/null 2>&1); check "an entry whose line is still there keeps passing on a change that does not touch it" 0 "$?"

# 7. what a line entry may not be
bad_entry() { # <description> <entry line>
	reset_to "$base"
	printf '%s\n' "$2" > "$repo/.secret-scan-allow"; commit_all "the list"
	local nb; nb="$(git -C "$repo" rev-parse HEAD)"
	echo more >> "$repo/README"; commit_all "touch"
	out="$(cd "$repo" && "$SCRIPT" "$nb" HEAD 2>&1)"; rc=$?
	check "$1" "1 1" "$rc $(printf '%s' "$out" | grep -c '::error::.secret-scan-allow line 1')"
}
bad_entry "an empty text is refused" "config/index.ts :: "
bad_entry "a blank text is refused" "config/index.ts ::    "
bad_entry "a directory with a line is refused" "config/ :: some line"
bad_entry "a wildcard in the path of a line entry is refused" "config/*.ts :: some line"
bad_entry "a path above the root is refused" "../config/index.ts :: some line"
bad_entry "an absolute path is refused" "/config/index.ts :: some line"
bad_entry "the header of a private key is refused, whole or in a longer text" "config/index.ts :: -----BEGIN PRIVATE KEY-----"
bad_entry "the header of an RSA key is refused too" "config/index.ts ::   key: |-----BEGIN RSA PRIVATE KEY-----"
# the characters of a pattern are fine in the text, which is never compiled
reset_to "$base"
text="a * b [c] (d) \$e ^f | g"
printf 'config/index.ts :: %s\n' "$text" > "$repo/.secret-scan-allow"; printf '%s\n' "$text" > "$repo/config/index.ts"; commit_all "text with pattern characters"
nb="$(git -C "$repo" rev-parse HEAD)"; echo x >> "$repo/README"; commit_all touch
(cd "$repo" && "$SCRIPT" "$nb" HEAD >/dev/null 2>&1); check "pattern characters in the text are literal and fine" 0 "$?"
# a line entry does not exempt a file: the same path with another text
reset_to "$base"
printf 'export const c = {\n%s\n};\n' "$other" > "$repo/config/index.ts"; commit_all "another text, same file"
(cd "$repo" && "$SCRIPT" "$base" HEAD >/dev/null 2>&1); check "an entry for one line is not an entry for its file" 1 "$?"
# and a whole-file entry next to it still works
reset_to "$base"
printf 'config/index.ts :: %s\nconfig/wide.ts\n' "$fp" > "$repo/.secret-scan-allow"; printf '%s\n' "$other" > "$repo/config/wide.ts"; printf '%s\n' "$fp" > "$repo/config/index.ts"; commit_all "both kinds"
wb="$(git -C "$repo" rev-parse HEAD)"; printf '%s\n' "$other" >> "$repo/config/wide.ts"; printf '%s\n' "$fp" >> "$repo/config/index.ts"; commit_all "more"
(cd "$repo" && "$SCRIPT" "$wb" HEAD >/dev/null 2>&1); check "a file entry and a line entry live side by side" 0 "$?"

echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
