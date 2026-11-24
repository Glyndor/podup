#!/usr/bin/env bash
#
# What the secret scan reads beyond the net diff: every commit and every commit message, a key
# whose header is not at the start of a line, and input that is not text. The credential-shaped
# strings are
# assembled at run time, so this file does not trip the check it tests, and no
# value is ever printed by the script.
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
echo base > "$repo/README"; gitc add -A; gitc commit -q -m base
base="$(git -C "$repo" rev-parse HEAD)"
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

token="gh$(printf 'p')_$(rep a 36)"
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


# Input that is not text. A NUL or a byte that is not valid UTF-8 must not make the scan stop
# reading: the file with the NUL sorts before the file with the credential, and the invalid
# byte is in the very line that holds it.
git -C "$repo" reset -q --hard "$base"
printf 'before\0after\n' > "$repo/0-nul.bin"; printf 't=%s\n' "$token" > "$repo/z.txt"
gitc add -A; gitc commit -q -m "nul first"
out="$(cd "$repo" && "$SCRIPT" "$base" HEAD 2>&1)"; rc=$?
check "a file with a NUL that sorts before a credential does not hide it" "1 1" "$rc $(printf '%s' "$out" | grep -c 'z.txt:1 GitHub token')"
git -C "$repo" reset -q --hard "$base"
printf 't=\377%s\n' "$token" > "$repo/bad.txt"; gitc add -A; gitc commit -q -m "invalid utf-8"
out="$(cd "$repo" && "$SCRIPT" "$base" HEAD 2>&1)"; rc=$?
check "an invalid UTF-8 byte in the line of a credential does not hide it" "1 1" "$rc $(printf '%s' "$out" | grep -c 'bad.txt:1 GitHub token')"
git -C "$repo" reset -q --hard "$base"
printf 'x\0y=%s\n' "$token" > "$repo/mixed.txt"; gitc add -A; gitc commit -q -m "nul in the line"
out="$(cd "$repo" && "$SCRIPT" "$base" HEAD 2>&1)"; rc=$?
check "a NUL in the line of a credential does not hide it" "1 1" "$rc $(printf '%s' "$out" | grep -c 'mixed.txt:1 GitHub token')"
git -C "$repo" reset -q --hard "$base"
printf '\377\376 not text at all \200\n\0\0\n' > "$repo/noise.bin"; gitc add -A; gitc commit -q -m "noise"
(cd "$repo" && "$SCRIPT" "$base" HEAD >/dev/null 2>&1); check "a file of noise with no credential passes" 0 "$?"
printf 'x\n\0\n' > "$repo/m.bin"; printf 'k=%s\n' "$token" > "$repo/m.txt"; gitc add -A; gitc commit -q -m "message too"
git -C "$repo" commit -q --amend -m "$(printf 'msg\377 %s' "$token")" 2>/dev/null
out="$(cd "$repo" && "$SCRIPT" "$base" HEAD 2>&1)"; rc=$?
check "a message with an invalid byte and a credential is read" 1 "$rc"

# A grep that fails is not a grep that found nothing. A fake grep that exits 2 for the pattern
# of one kind of token must stop the scan: the same clean line passes with the real one.
shim="$WORK/shim"; mkdir -p "$shim"
real_grep="$(command -v grep)"
printf '#!/bin/sh\ncase "$*" in *"gh[pousr]_"*) exit 2 ;; esac\nexec %s "$@"\n' "$real_grep" > "$shim/grep"; chmod +x "$shim/grep"
git -C "$repo" reset -q --hard "$base"
printf 'just text\n' > "$repo/clean.txt"; gitc add -A; gitc commit -q -m clean
out="$(cd "$repo" && PATH="$shim:$PATH" "$SCRIPT" "$base" HEAD 2>&1)"; rc=$?
check "a grep that fails stops the scan, with an error and no clean verdict" "2 0" "$rc $(printf '%s' "$out" | grep -c 'no credential-shaped lines added')"
(cd "$repo" && "$SCRIPT" "$base" HEAD >/dev/null 2>&1); check "and the same clean change passes with a grep that works" 0 "$?"


scan() { # <line>: commit it alone on top of the base and scan it
	git -C "$repo" reset -q --hard "$base"
	printf '%s\n' "$1" > "$repo/probe.txt"; gitc add -A; gitc commit -q -m probe
	(cd "$repo" && "$SCRIPT" "$base" HEAD 2>&1)
}

# The two loops that read the candidate lines, and the fake grep that fails only for their own
# pattern: the header search (a pattern that ends the header) and the assignment search.
printf '#!/bin/sh\ncase "$*" in *"BLOCK)?-----)") exit 2 ;; esac\nexec %s "$@"\n' "$real_grep" > "$shim/grep"
git -C "$repo" reset -q --hard "$base"; printf 'just text\n' > "$repo/clean.txt"; gitc add -A; gitc commit -q -m clean
out="$(cd "$repo" && PATH="$shim:$PATH" "$SCRIPT" "$base" HEAD 2>&1)"; rc=$?
check "a grep that fails in the search for key headers stops the scan" "2 0" "$rc $(printf '%s' "$out" | grep -c 'no credential-shaped lines added')"
printf '#!/bin/sh\ncase "$*" in *"passw(or)?d"*) exit 2 ;; esac\nexec %s "$@"\n' "$real_grep" > "$shim/grep"
out="$(cd "$repo" && PATH="$shim:$PATH" "$SCRIPT" "$base" HEAD 2>&1)"; rc=$?
check "a grep that fails in the search for assignments stops the scan" "2 0" "$rc $(printf '%s' "$out" | grep -c 'no credential-shaped lines added')"

# The line after a header is looked for among every record of that path and number: the pair
# turns up once for each commit that touches it. The header and the body are added by one
# commit and taken out by the next, and the final file has other text on both lines.
git -C "$repo" reset -q --hard "$base"
printf 'plain one\nplain two\n' > "$repo/p.txt"; gitc add -A; gitc commit -q -m "plain"
printf 'data = ["%s",\n  "%s",\n]\n' "$H" "$body" > "$repo/p.txt"; gitc add -A; gitc commit -q -m "key"
printf 'plain one\nplain two\n' > "$repo/p.txt"; gitc add -A; gitc commit -q -m "back to plain"
out="$(cd "$repo" && "$SCRIPT" "$base" HEAD 2>&1)"; rc=$?
check "a key whose header and body were added together and taken out later is found" "1 1" "$rc $(printf '%s' "$out" | grep -c 'p.txt:1 private key spread over lines')"

# An assignment is not only name, colon or equals sign and value: a tab, a type and an
# operator of several characters are assignments too. Each is paired with a form that
# passes (a variable that names a file, a sample value).
v="$(rep k 40)"
tab="$(printf '\t')"
for form in \
	"token${tab}=${tab}\"$v\"" \
	"const TOKEN: &str = \"$v\";" \
	"const API_KEY: &'static str = \"$v\";" \
	"let token: String = \"$v\".to_string();" \
	"token: string = \"$v\";" \
	"token := \"$v\"" \
	"token => \"$v\"," \
	"password:${tab}'$v'" \
	"secret: Vec<u8> = \"$v\";"; do
	out="$(scan "$form")"; rc=$?
	check "an assignment written as [$(printf '%s' "$form" | sed "s/$v/VALUE/")] is found" "1 1" "$rc $(printf '%s' "$out" | grep -c 'long value assigned to a secret-like name')"
done
for form in \
	"const TOKEN_FILE: &str = \"/run/secrets/$v\";" \
	"token_path${tab}:= \"$v\"" \
	"const TOKEN: &str = \"your-$v\";" \
	"token: string = \"example_$v\";" \
	"let n: u32 = 12;"; do
	out="$(scan "$form")"; rc=$?
	check "a sample or a path written as [$(printf '%s' "$form" | sed "s/$v/VALUE/")] passes" 0 "$rc"
done

# An excuse for one assignment does not reach into the next on the same line. A comma or a
# semicolon belongs to a type only inside brackets, so the exempt name or the sample word of
# the first assignment cannot cover a real credential in the second, and a plain assignment is
# read on its own even when a typed match overlaps it.
for form in \
	"token_name: foo, password: \"$v\"" \
	"token_url: str, password=\"$v\"" \
	"token: $v, cert_path: /$v" \
	"token_file: \"your-$v\"; secret = \"$v\"" \
	"let api_key_path: Map<String, u8> = x; let token = \"$v\";" \
	"token_name: a, b, c = d, password := $v" \
	"token_path: Arc<Mutex<Option<Vec<u8>>>> = \"x\", password = \"$v\"" \
	"token_path: Vec<u8>, password: Arc<Mutex<Option<Vec<u8>>>> = \"$v\"" \
	"secret_file: Cow<'static, [u8]>, token = \"$v\"" \
	"token_url: List[str], password: Dict[str, List[str]] = \"$v\"" \
	"password_file: Vec<a> , token: Vec<b> = \"$v\"" \
	"secret_path: Map<a;b> ; api_key: Map<c> = \"$v\""; do
	out="$(scan "$form")"; rc=$?
	check "a real credential after an excused one is found in [$(printf '%s' "$form" | sed "s/$v/VALUE/g")]" "1 1" "$rc $(printf '%s' "$out" | grep -c 'long value assigned to a secret-like name')"
done
for form in \
	"const API_KEY_FILE: [u8; 32] = \"$v\";" \
	"const TOKEN_PATH: Map<String, String> = \"$v\";" \
	"token_url: str, cert_path: \"$v\""; do
	out="$(scan "$form")"; rc=$?
	check "a name that holds a path keeps its excuse, a bracketed type included, in [$(printf '%s' "$form" | sed "s/$v/VALUE/g")]" 0 "$rc"
done

# What the pattern of the first version accepted by accident: a quote with a backslash in front
# of it (a JSON string, generated YAML) and a tab written as a backslash and a t in the text.
bs="\\"
for form in \
	"token=${bs}\"$v${bs}\"" \
	"password: ${bs}\"$v${bs}\"" \
	"token:${bs}t$v" \
	"${bs}\"token${bs}\":${bs}t${bs}\"$v${bs}\"" \
	"{${bs}\"secret${bs}\": ${bs}\"$v${bs}\"}" \
	"password=${bs}t${bs}t'$v'"; do
	out="$(scan "$form")"; rc=$?
	check "a form with a backslash is found in [$(printf '%s' "$form" | sed "s/$v/VALUE/g")]" "1 1" "$rc $(printf '%s' "$out" | grep -c 'long value assigned to a secret-like name')"
done
for form in \
	"token_file=${bs}\"/run/secrets/$v${bs}\"" \
	"password_url:${bs}t$v" \
	"token=${bs}\"your-$v${bs}\"" \
	"token:${bs}tfake_$v"; do
	out="$(scan "$form")"; rc=$?
	check "an excused form with a backslash passes in [$(printf '%s' "$form" | sed "s/$v/VALUE/g")]" 0 "$rc"
done

# Brackets in a type nest. The credential is found whatever the depth of the type, and a name
# that holds a path keeps its excuse with a nested type; a comma outside the brackets still ends
# the type, so the credential after it is the second assignment's own.
for form in \
	"token: ClassVar[Optional[str]] = \"$v\"" \
	"secret: Cow<'static, [u8]> = \"$v\";" \
	"token: ClassVar[Optional[List[str]]] = \"$v\"" \
	"token: Arc<Mutex<Option<Vec<u8>>>> = \"$v\"" \
	"secret: Rc<RefCell<Option<Vec<Box<[u8]>>>>> = \"$v\";" \
	"password: ClassVar[Optional[List[Dict[str, List[str]]]]] = \"$v\"" \
	"let token: Mutex<HashMap<String, Vec<[u8; 32]>>> = \"$v\";" \
	"let password: HashMap<String, Vec<u8>> = \"$v\";" \
	"token_path: Dict[str, List[str]] = \"x\", password = \"$v\""; do
	out="$(scan "$form")"; rc=$?
	check "a nested type does not hide an assignment in [$(printf '%s' "$form" | sed "s/$v/VALUE/g")]" "1 1" "$rc $(printf '%s' "$out" | grep -c 'long value assigned to a secret-like name')"
done
for form in \
	"token_path: Dict[str, List[str]] = \"$v\"" \
	"let api_key_file: HashMap<String, Vec<u8>> = \"$v\";" \
	"secret_url: Cow<'static, [u8]> = \"$v\";" \
	"token_path: Arc<Mutex<Option<Vec<u8>>>> = \"$v\"" \
	"secret_file: Rc<RefCell<Option<Vec<Box<[u8]>>>>> = \"$v\";"; do
	out="$(scan "$form")"; rc=$?
	check "a nested type with a name that holds a path passes in [$(printf '%s' "$form" | sed "s/$v/VALUE/g")]" 0 "$rc"
done

# The mark that skips a line is the author's own, so the log says where it was used and never what
# the line held; and it skips nothing in the message of a commit.
mark="secret-scan"":allow"
git -C "$repo" reset -q --hard "$base"
printf 'one = 1\nt=%s # %s\n' "$token" "$mark" > "$repo/marked.txt"; printf 'two = 2\nv=%s // %s\n' "$token" "$mark" > "$repo/marked2.txt"; gitc add -A; gitc commit -q -m marks
out="$(cd "$repo" && "$SCRIPT" "$base" HEAD 2>&1)"; rc=$?
check "a line with the mark passes" 0 "$rc"
check "and the log names the file and line of each, counted" "1 1 1" "$(printf '%s' "$out" | grep -c 'skipped 2 line(s)') $(printf '%s' "$out" | grep -c 'marked.txt:2') $(printf '%s' "$out" | grep -c 'marked2.txt:2')"
check "and does not print what they held" 0 "$(printf '%s' "$out" | grep -cF "$token")"
git -C "$repo" reset -q --hard "$base"
printf 'ordinary\n' > "$repo/h.txt"; gitc add -A; gitc commit -q -m "a note that carries $token and the mark $mark"
out="$(cd "$repo" && "$SCRIPT" "$base" HEAD 2>&1)"; rc=$?
check "the mark does not skip a credential in the message of a commit" "1 1" "$rc $(printf '%s' "$out" | grep -c 'GitHub token')"
check "and a message is not counted among the skipped lines" 0 "$(printf '%s' "$out" | grep -c 'skipped')"
# a header that carries the mark is still read for the key below it
git -C "$repo" reset -q --hard "$base"
printf 'data = ["%s", // %s\n  "%s",\n]\n' "$H" "$mark" "$body" > "$repo/k.txt"; gitc add -A; gitc commit -q -m "a marked header"
out="$(cd "$repo" && "$SCRIPT" "$base" HEAD 2>&1)"; rc=$?
check "a marked header does not hide the body of a key below it" "1 1" "$rc $(printf '%s' "$out" | grep -c 'k.txt:1 private key spread over lines')"

# Bytes that are not text, in the same file: a NUL and a 0xFF byte, in a file whose name sorts before
# the file that holds the token. The scan reads bytes (no locale, no grep that takes the stream for
# binary) and a failure to read is an error and never a scan that found nothing.
git -C "$repo" reset -q --hard "$base"
printf 'before\0\377after\n\377\377\0\n' > "$repo/0-bytes.bin"; printf 't=%s\n' "$token" > "$repo/z.txt"; gitc add -A; gitc commit -q -m "bytes first"
out="$(cd "$repo" && LC_ALL=en_US.UTF-8 LANG=en_US.UTF-8 "$SCRIPT" "$base" HEAD 2>&1)"; rc=$?
check "a file with a NUL and a 0xFF that sorts first does not hide a token, whatever the locale" "1 1" "$rc $(printf '%s' "$out" | grep -c 'z.txt:1 GitHub token')"
git -C "$repo" reset -q --hard "$base"
printf 't=%s\n' "$token" > "$repo/0-first.txt"; printf 'x\0\377\n' > "$repo/z.bin"; gitc add -A; gitc commit -q -m "token first"
out="$(cd "$repo" && LC_ALL=C.UTF-8 "$SCRIPT" "$base" HEAD 2>&1)"; rc=$?
check "and the same with the token in the file that sorts first" "1 1" "$rc $(printf '%s' "$out" | grep -c '0-first.txt:1 GitHub token')"

# A root commit: a branch with no history in common whose first commit adds a token and whose second
# commit takes it out, merged. The root commit has no parent, so it is read against the empty tree;
# what the net diff and the merge show is a tree without the token.
trunk=mainline
git -C "$repo" checkout -q -f -B "$trunk" "$base"
git -C "$repo" checkout -q --orphan unrelated
git -C "$repo" rm -rq --cached . >/dev/null 2>&1; find "$repo" -mindepth 1 -maxdepth 1 ! -name .git -exec rm -rf {} +
printf 't=%s\n' "$token" > "$repo/root.txt"; gitc add -A; gitc commit -q -m "a root commit with a token"
printf 'clean\n' > "$repo/root.txt"; gitc add -A; gitc commit -q -m "and one that takes it out"
git -C "$repo" checkout -q -f "$trunk"
git -C "$repo" -c user.name=t -c user.email=t@x -c commit.gpgSign=false merge -q --allow-unrelated-histories --no-edit unrelated >/dev/null 2>&1
out="$(cd "$repo" && "$SCRIPT" "$base" HEAD 2>&1)"; rc=$?
check "a token added by a root commit and taken out by the next is found" "1 1" "$rc $(printf '%s' "$out" | grep -c 'root.txt:1 GitHub token')"
check "and the merge that brings it in does not hide it" 1 "$(git -C "$repo" rev-list --merges "$base..HEAD" | wc -l | tr -d ' ')"
check "and no value is printed" 0 "$(printf '%s' "$out" | grep -cF "$token")"
git -C "$repo" checkout -q -f --detach "$base"; git -C "$repo" branch -q -D unrelated "$trunk"

echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
