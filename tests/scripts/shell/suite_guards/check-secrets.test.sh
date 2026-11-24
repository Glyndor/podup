#!/usr/bin/env bash
#
# Tests for .github/scripts/hygiene/check-secrets.sh. The credential-shaped strings are
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
# The two names the release workflow reads its signing keys from.
flag "the Ed25519 release signing key" "GLYNDOR_RELEASE_ED25519_KEY: $(rep e 43)=" "secret-like name"
flag "the ML-DSA-65 release signing key" "GLYNDOR_RELEASE_MLDSA65_KEY: $(rep f 43)=" "secret-like name"
out="$(scan "GLYNDOR_RELEASE_MLDSA65_KEY: \${{ secrets.GLYNDOR_RELEASE_MLDSA65_KEY }}")"
check "the ML-DSA-65 key name with a reference to the CI secret passes" 0 "$?"

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
out="$(scan "api_key = example-$(rep 0 40)")"
check "a value that starts with a sample word and a hyphen passes" 0 "$?"
out="$(scan "password: \"dummy_$(rep 7 40)\"")"
check "a value with a sample word before an underscore passes" 0 "$?"

# The words that excuse a sample are read in the value, as a word of their own: not in the
# name, not in a comment, and not inside a run of letters of a key.
sample() { # <description> <line>: refused, and named as a long value
	local out rc
	out="$(scan "$2")"; rc=$?
	check "$1 fails" 1 "$rc"
	check "$1 is named" 1 "$(printf '%s' "$out" | grep -c 'probe.txt:1 long value assigned to a secret-like name')"
}
sample "a key whose value starts with a sample word and goes on in letters" "GLYNDOR_RELEASE_ED25519_KEY=example$(rep A 36)="
sample "a key with a comment that says example" "GLYNDOR_RELEASE_ED25519_KEY: $(rep e 43)= # an example of the format"
sample "a key with a comment that says fake" "api_key = \"$(rep k 40)\" // not a fake"
sample "a value that holds fake between letters" "api_key = \"$(rep a 12)fake$(rep b 30)\""
sample "a sample word in the name and not in the value" "example_api_key = \"$(rep c 40)\""
sample "a key whose name says file and value does not" "api_key = \"$(rep d 40)\" # was api_key_file = x"

# A secret that was already in the base is not a reason to fail this change.
printf 'old_token = "%s"\n' "$(rep z 40)" > "$repo/old.txt"; gitc add -A; gitc commit -q -m "old"
base2="$(git -C "$repo" rev-parse HEAD)"
echo more >> "$repo/README"; gitc add -A; gitc commit -q -m "touch"
(cd "$repo" && "$SCRIPT" "$base2" HEAD >/dev/null 2>&1); check "an old line in the base is not rescanned" 0 "$?"

# A file whose content is already in the tree of the base adds nothing to the history, so
# moving or copying it, an exempt file into another folder or a key that was already there
# into a new path, is not a reason to fail. The fixture repository asks for rename
# detection itself, since a machine's own configuration can turn it off. A moved file
# that gains a line is a new file as far as that line goes, and is read whole.
git -C "$repo" config diff.renames true
mkdir -p "$repo/moved"; gitc mv old.txt moved/old.txt; gitc commit -q -m "move the old file"
out="$( (cd "$repo" && "$SCRIPT" "$base2" HEAD 2>&1) )"; rc=$?
check "a file moved unchanged adds nothing and passes" "0 1" "$rc $(printf '%s' "$out" | grep -c 'no credential-shaped lines added')"
gitc reset -q --hard "$base2"
mkdir -p "$repo/copied"; cp "$repo/old.txt" "$repo/copied/old.txt"; gitc add -A; gitc commit -q -m "copy the old file"
(cd "$repo" && "$SCRIPT" "$base2" HEAD >/dev/null 2>&1); check "a file copied unchanged passes" 0 "$?"
gitc reset -q --hard "$base2"
mkdir -p "$repo/moved"; gitc mv old.txt moved/old.txt
printf 'second_token = "%s"\n' "$(rep y 40)" >> "$repo/moved/old.txt"; gitc add -A; gitc commit -q -m "move the old file and add a line"
out="$( (cd "$repo" && "$SCRIPT" "$base2" HEAD 2>&1) )"; rc=$?
check "the same move with one line added that holds a credential fails" 1 "$rc"
check "and the file is read whole and named at its new path" 1 "$(printf '%s' "$out" | grep -c 'moved/old.txt:1 long value assigned to a secret-like name')"
check "and the added line is named too" 1 "$(printf '%s' "$out" | grep -c 'moved/old.txt:2 long value assigned to a secret-like name')"
check "and no value is printed" 0 "$(printf '%s' "$out" | grep -cF -e "$(rep z 40)" -e "$(rep y 40)")"
gitc reset -q --hard "$base2"
base3="$(git -C "$repo" rev-parse HEAD)"
gitc mv README README.md; gitc commit -q -m "rename a harmless file"
(cd "$repo" && "$SCRIPT" "$base3" HEAD >/dev/null 2>&1); check "a harmless file moved the same way passes" 0 "$?"
gitc reset -q --hard "$base2"

# A repository lists the paths that hold throwaway keys on purpose. The list counts
# once it is on the base branch, so it is committed to the base before the change.
git -C "$repo" reset -q --hard "$base"
printf '# test keys (throwaway only), no * here\n\nfixtures/\n' > "$repo/.secret-scan-allow"
gitc add -A; gitc commit -q -m "list the fixture paths"
listed="$(git -C "$repo" rev-parse HEAD)"
mkdir -p "$repo/fixtures"
printf -- '-----BEGIN %s-----\n%s\n' 'PRIVATE KEY' "$(rep M 40)" > "$repo/fixtures/test.pem"
gitc add -A; gitc commit -q -m "fixture"
(cd "$repo" && "$SCRIPT" "$listed" HEAD >/dev/null 2>&1); check "a path listed in .secret-scan-allow on the base is skipped" 0 "$?"
printf -- '-----BEGIN %s-----\n' 'PRIVATE KEY' > "$repo/elsewhere.pem"; gitc add -A; gitc commit -q -m "elsewhere"
out="$( (cd "$repo" && "$SCRIPT" "$listed" HEAD 2>&1) )"; rc=$?
check "a key outside the listed paths still fails" 1 "$rc"
check "and it is refused as a key block" 1 "$(printf '%s' "$out" | grep -c 'elsewhere.pem:1 private key block')"

# The path a line is reported under comes from git, never from the text of the patch.
# A file that adds a line reading "++ b/fixtures/x" makes the patch carry "+++ b/fixtures/x"
# inside a hunk, which is not a header: the credential after it stays in the file that
# added it, and is not taken for a line of a listed path.
git -C "$repo" reset -q --hard "$listed"
printf -- '++ b/fixtures/x\nAWS_KEY=AKI%s\n' "A$(rep Q 16)" > "$repo/evil.txt"
gitc add -A; gitc commit -q -m "a line that looks like a header"
out="$( (cd "$repo" && "$SCRIPT" "$listed" HEAD 2>&1) )"; rc=$?
check "a line that looks like the header of a listed file does not move what follows to it" 1 "$rc"
check "and the credential is named at the file that added it" 1 "$(printf '%s' "$out" | grep -c 'evil.txt:2 AWS access key')"
git -C "$repo" reset -q --hard "$listed"
mkdir -p "$repo/fixtures"
printf -- '++ b/elsewhere\nAWS_KEY=AKI%s\n' "A$(rep Q 16)" > "$repo/fixtures/real.txt"
gitc add -A; gitc commit -q -m "the same text in a file that is listed"
(cd "$repo" && "$SCRIPT" "$listed" HEAD >/dev/null 2>&1); check "the same two lines in a file that is listed pass" 0 "$?"

# A name with a backslash in it is reported as it is, so a list written for it matches.
git -C "$repo" reset -q --hard "$listed"
mkdir -p "$repo/fixtures"
printf 'AWS_KEY=AKI%s\n' "A$(rep Q 16)" > "$repo/fixtures/a\\tb.txt"
gitc add -A; gitc commit -q -m "a name with a backslash"
(cd "$repo" && "$SCRIPT" "$listed" HEAD >/dev/null 2>&1); check "a listed file whose name holds a backslash passes" 0 "$?"
git -C "$repo" reset -q --hard "$base"
printf 'AWS_KEY=AKI%s\n' "A$(rep Q 16)" > "$repo/a\\tb.txt"
gitc add -A; gitc commit -q -m "a name with a backslash, not listed"
out="$( (cd "$repo" && "$SCRIPT" "$base" HEAD 2>&1) )"
check "an unlisted one is named with its backslash" 1 "$(printf '%s' "$out" | grep -c -F 'a\tb.txt:1 AWS access key')"

# A file is diffed by its own name, taken literally: a file called "*" would otherwise
# pull the lines of every other file under its label.
git -C "$repo" reset -q --hard "$listed"
mkdir -p "$repo/fixtures"
printf 'AWS_KEY=AKI%s\n' "A$(rep Q 16)" > "$repo/fixtures/key.txt"
printf 'hello\n' > "$repo/*"
gitc add -A; gitc commit -q -m "a file called star beside a listed one"
(cd "$repo" && "$SCRIPT" "$listed" HEAD >/dev/null 2>&1); check "a file named * does not take the lines of a listed file under its own name" 0 "$?"

# A file name is part of what the scan prints and of what it compares against the list, so
# a name with a line break in it could forge a record of a listed path: "x\nfixtures/y"
# plus ":1:" and the credential reads as the line of fixtures/y. Names with a control
# character are refused whole, before any record is made; a name with a space or a
# letter outside ASCII is a name like any other.
git -C "$repo" reset -q --hard "$listed"
mkdir -p "$repo/$(printf 'x\nfixtures')"
printf 'AWS_KEY=AKI%s\n' "A$(rep Q 16)" > "$repo/$(printf 'x\nfixtures/y')"
gitc add -A; gitc commit -q -m "a name that forges a record"
out="$( (cd "$repo" && "$SCRIPT" "$listed" HEAD 2>&1) )"; rc=$?
check "a file name with a line break is refused" 1 "$rc"
check "and the refusal says why" 1 "$(printf '%s' "$out" | grep -c '::error::a changed file has a control character in its name, so nothing was scanned')"
# The refusal ends the scan: a line reported under a name that cannot be trusted
# would be a record somebody else chose the path of.
check "and nothing is reported under that name" 0 "$(printf '%s' "$out" | grep -c 'AWS access key')"
git -C "$repo" reset -q --hard "$listed"
printf 'AWS_KEY=AKI%s\n' "A$(rep Q 16)" > "$repo/$(printf 'a\tb.txt')"
gitc add -A; gitc commit -q -m "a name with a tab"
out="$( (cd "$repo" && "$SCRIPT" "$listed" HEAD 2>&1) )"; rc=$?
check "a file name with a tab is refused, and nothing is reported under it" "1 1 0" "$rc $(printf '%s' "$out" | grep -c 'control character in its name') $(printf '%s' "$out" | grep -c 'AWS access key')"
git -C "$repo" reset -q --hard "$listed"
printf 'nothing to see\n' > "$repo/a b é.txt"
gitc add -A; gitc commit -q -m "a name with a space and a letter outside ASCII"
out="$( (cd "$repo" && "$SCRIPT" "$listed" HEAD 2>&1) )"; rc=$?
check "a file name with a space and a letter outside ASCII is scanned like any other" "0 1" "$rc $(printf '%s' "$out" | grep -c 'no credential-shaped lines added')"

# The allow marker is looked for in the text of a line, and the skip words of the
# generic rule too: neither is read out of the path, which the author of a change chooses.
git -C "$repo" reset -q --hard "$base"
printf 'AWS_KEY=AKI%s\n' "A$(rep Q 16)" > "$repo/secret-scan:allow.txt"
printf 'release_secret = "%s"\n' "$(rep c 40)" > "$repo/fixture-example-dummy.env"
gitc add -A; gitc commit -q -m "names with the marker and the skip words"
out="$( (cd "$repo" && "$SCRIPT" "$base" HEAD 2>&1) )"; rc=$?
check "a path that holds the allow marker does not skip its lines" 1 "$(printf '%s' "$out" | grep -c 'secret-scan:allow.txt:1 AWS access key')"
check "a path that holds a skip word does not skip its lines" 1 "$(printf '%s' "$out" | grep -c 'fixture-example-dummy.env:1 long value assigned to a secret-like name')"
check "and both fail the scan" 1 "$rc"

# The list is compared with the path alone: an entry that happens to be the text of a
# line does not skip it. A file below a listed directory is skipped, and a file that
# merely has the listed name as its text is not.
git -C "$repo" reset -q --hard "$base"
printf 'AKIAQQQQ/\n' > "$repo/.secret-scan-allow"
gitc add -A; gitc commit -q -m "a list with one directory"
textual="$(git -C "$repo" rev-parse HEAD)"
printf 'AWS_KEY=AKI%s\n' "A$(rep Q 16)" > "$repo/c.txt"
gitc add -A; gitc commit -q -m "a credential"
out="$( (cd "$repo" && "$SCRIPT" "$textual" HEAD 2>&1) )"; rc=$?
check "a listed name that is the text of a line and not its path does not skip it" "1 1" "$rc $(printf '%s' "$out" | grep -c 'c.txt:1 AWS access key')"
git -C "$repo" reset -q --hard "$textual"
mkdir -p "$repo/AKIAQQQQ"; printf 'AWS_KEY=AKI%s\n' "A$(rep Q 16)" > "$repo/AKIAQQQQ/c.txt"
gitc add -A; gitc commit -q -m "the same line in a listed directory"
(cd "$repo" && "$SCRIPT" "$textual" HEAD >/dev/null 2>&1); check "the same entry skips a file below the directory it names" 0 "$?"

# What an entry exempts is what it says. A file entry is that file and no other; a
# directory entry (it ends in a slash) is what lies below it, and not a directory that
# only starts with the same letters. Each case plants a credential in one path and
# is paired with the case that lists that path and passes.
path_case() { # <list> <path> <want: skipped|scanned> <description>
	local out rc
	git -C "$repo" reset -q --hard "$base"
	printf '%s\n' "$1" > "$repo/.secret-scan-allow"
	gitc add -A; gitc commit -q -m "a list"
	local listed_base; listed_base="$(git -C "$repo" rev-parse HEAD)"
	mkdir -p "$repo/$(dirname "$2")"
	printf 'AWS_KEY=AKI%s\n' "A$(rep Q 16)" > "$repo/$2"
	gitc add -A; gitc commit -q -m "a credential"
	out="$( (cd "$repo" && "$SCRIPT" "$listed_base" HEAD 2>&1) )"; rc=$?
	if [ "$3" = skipped ]; then
		check "$4" "0 1" "$rc $(printf '%s' "$out" | grep -c 'no credential-shaped lines added')"
	else
		check "$4" "1 1" "$rc $(printf '%s' "$out" | grep -c "$2:1 AWS access key")"
	fi
}
path_case 'tests/keys/test.pem' 'tests/keys/test.pem' skipped "a file entry skips that file"
path_case 'tests/keys/test.pem' 'tests/keys/other.pem' scanned "a file entry does not skip the file beside it"
path_case 'tests/keys/test.pem' 'tests/keys/test.pem.bak' scanned "a file entry does not skip a file that starts with its name"
path_case 'tests/keys' 'tests/keys/test.pem' scanned "an entry without a slash is not a directory"
path_case 'tests/keys/' 'tests/keys/deep/er/test.pem' skipped "a directory entry skips every file below it"
path_case 'tests/keys/' 'tests/keys2/test.pem' scanned "a directory entry does not skip a directory that starts with its name"
path_case 'tests/keys/' 'tests/key' scanned "a directory entry does not skip the file named like it without the slash"
path_case 'tests/keys/' 'x/tests/keys/test.pem' scanned "a directory entry is anchored at the root"

# What git shows of a file does not depend on the attributes or the drivers of the tree being
# judged: a text file marked binary, or given a text conversion or an external diff, is
# still read as the text it is. Each case has a twin without the credential that passes.
attr_case() { # <description> <attribute line> [git config key=value]
	local out rc
	git -C "$repo" reset -q --hard "$base"; git -C "$repo" config --unset-all diff.drv.textconv 2>/dev/null; git -C "$repo" config --unset-all diff.drv.command 2>/dev/null
	[ -z "${3:-}" ] || git -C "$repo" config "${3%%=*}" "${3#*=}"
	printf '%s\n' "$2" > "$repo/.gitattributes"
	printf 'AWS_KEY=AKI%s\n' "A$(rep Q 16)" > "$repo/k.txt"
	gitc add -A; gitc commit -q -m "$1"
	out="$( (cd "$repo" && "$SCRIPT" "$base" HEAD 2>&1) )"; rc=$?
	check "$1: the credential fails the scan" 1 "$rc"
	check "$1: and is named at its file" 1 "$(printf '%s' "$out" | grep -c 'k.txt:1 AWS access key')"
	printf 'nothing to see\n' > "$repo/k.txt"; gitc add -A; gitc commit -q --amend -m "$1, without it"
	out="$( (cd "$repo" && "$SCRIPT" "$base" HEAD 2>&1) )"; rc=$?
	check "$1: the same tree without the credential passes" "0 1" "$rc $(printf '%s' "$out" | grep -c 'no credential-shaped lines added')"
	git -C "$repo" config --unset-all diff.drv.textconv 2>/dev/null; git -C "$repo" config --unset-all diff.drv.command 2>/dev/null
}
attr_case "a file marked -diff" '* -diff'
attr_case "a file marked binary" '* binary'
attr_case "a file with a text conversion" '* diff=drv' "diff.drv.textconv=echo converted #"
attr_case "a file with an external diff" '* diff=drv' "diff.drv.command=true"

# A file that is binary for real, with NUL bytes in it, is read as the text it also is: a
# credential inside one is named, and an image with none passes. Neither depends on a fake
# git: this is what git itself shows with --text.
git -C "$repo" reset -q --hard "$base"
printf '\211PNG\r\n\032\n\000\000\000\rIHDR\000\000\001\000\n' > "$repo/logo.png"
printf 'GH_TOKEN=gh%s_%s\n\000\001\377' "p" "$(rep A 36)" >> "$repo/logo.png"
gitc add -A; gitc commit -q -m "an image with a token inside"
out="$( (cd "$repo" && "$SCRIPT" "$base" HEAD 2>&1) )"; rc=$?
check "a binary file with a credential inside is refused, and named" "1 1" "$rc $(printf '%s' "$out" | grep -c 'logo.png:[0-9]* GitHub token')"
git -C "$repo" reset -q --hard "$base"
printf '\211PNG\r\n\032\n\000\000\000\rIHDR\000\000\001\000\n' > "$repo/logo.png"
printf 'nothing to see\n\000\001\377' >> "$repo/logo.png"
gitc add -A; gitc commit -q -m "an image with nothing inside"
out="$( (cd "$repo" && "$SCRIPT" "$base" HEAD 2>&1) )"; rc=$?
check "a binary file with no credential in it passes" "0 1" "$rc $(printf '%s' "$out" | grep -c 'no credential-shaped lines added')"

# A changed file that shows no lines to read stops the scan, whatever the reason. With
# --text git always shows the lines of a file, so the fake git answers a diff of a file with
# nothing, the way a driver or a future git could. A change with no content to read passes:
# an empty file, a file that is deleted, and one whose mode alone changed.
mkdir -p "$WORK/gitbin"
real_git="$(command -v git)"
cat > "$WORK/gitbin/git" <<GITEND
#!/bin/sh
case "\$*" in
	*"--no-ext-diff"*) case "\$FAKE" in nothing) ;; *) exec $real_git "\$@" ;; esac ;;
	*) exec $real_git "\$@" ;;
esac
GITEND
chmod +x "$WORK/gitbin/git"
git -C "$repo" reset -q --hard "$base"
printf 'AWS_KEY=AKI%s\n' "A$(rep Q 16)" > "$repo/k.txt"; gitc add -A; gitc commit -q -m "a file git will not show"
out="$( (cd "$repo" && FAKE=nothing PATH="$WORK/gitbin:$PATH" "$SCRIPT" "$base" HEAD 2>&1) )"; rc=$?
check "a changed file that shows nothing stops the scan" "1 1" "$rc $(printf '%s' "$out" | grep -c '::error::git showed no added lines for a changed file that is not empty, so nothing was scanned')"
check "and gives no clean verdict" 0 "$(printf '%s' "$out" | grep -c 'no credential-shaped lines added')"
out="$( (cd "$repo" && FAKE=real PATH="$WORK/gitbin:$PATH" "$SCRIPT" "$base" HEAD 2>&1) )"; rc=$?
check "the same fake git, passing the diff through, finds the credential by name" "1 1" "$rc $(printf '%s' "$out" | grep -c 'k.txt:1 AWS access key')"
printf 'text\n' > "$repo/mode.sh"; printf 'gone\n' > "$repo/gone.txt"; gitc add -A; gitc commit -q -m "more files"; moved_base="$(git -C "$repo" rev-parse HEAD)"
chmod +x "$repo/mode.sh"; : > "$repo/empty.txt"; gitc rm -q gone.txt; gitc add -A; gitc commit -q -m "a mode change, an empty file and a deletion"
out="$( (cd "$repo" && FAKE=nothing PATH="$WORK/gitbin:$PATH" "$SCRIPT" "$moved_base" HEAD 2>&1) )"; rc=$?
check "a mode change, an empty file and a deletion leave nothing to read and pass" "0 1" "$rc $(printf '%s' "$out" | grep -c 'no credential-shaped lines added')"
# A mode change of a file whose content the base has since dropped: the blob is not in the
# tree of the base, there is no hunk, and the content is the one the change started from.
git -C "$repo" reset -q --hard "$moved_base"
git -C "$repo" checkout -q -b stale
chmod +x "$repo/mode.sh"; gitc add -A; gitc commit -q -m "a mode change on an old base"
git -C "$repo" checkout -q -; gitc rm -q mode.sh; gitc commit -q -m "main drops the file"
tip="$(git -C "$repo" rev-parse HEAD)"
out="$( (cd "$repo" && "$SCRIPT" "$tip" stale 2>&1) )"; rc=$?
check "a mode change of content the base has dropped has nothing to read and passes" "0 1" "$rc $(printf '%s' "$out" | grep -c 'no credential-shaped lines added')"
git -C "$repo" branch -q -D stale

# A change cannot list its own exceptions: the same fixture with the list added by
# the change itself is scanned, and an expression that matches everything is no
# way to switch the scan off.
for list in 'fixtures/' 'fixtures/test.pem' 'a-plain-path'; do
	git -C "$repo" reset -q --hard "$base"
	mkdir -p "$repo/fixtures"
	printf -- '-----BEGIN %s-----\n' 'PRIVATE KEY' > "$repo/fixtures/test.pem"
	printf '%s\n' "$list" > "$repo/.secret-scan-allow"
	gitc add -A; gitc commit -q -m "fixture with its own list"
	out="$( (cd "$repo" && "$SCRIPT" "$base" HEAD 2>&1) )"; rc=$?
	check "a list the change brings itself ($list) is not read: it fails" 1 "$rc"
	check "and the key is named, not the list" 1 "$(printf '%s' "$out" | grep -c 'fixtures/test.pem:1 private key block')"
	check "and no complaint about the list is made" 0 "$(printf '%s' "$out" | grep -c 'secret-scan-allow line')"
done

# A line of the list that is not a plain path stops the scan, naming its line, whatever
# the change adds: a list that cannot be read as it is must not be taken for a scan that
# found nothing. Each rejection is paired with the same list holding a plain path in that
# line, which passes and skips the file it names.
listed_case() { # <the third line of the list>: sets out and rc for a credential in c.txt
	git -C "$repo" reset -q --hard "$base"
	printf '# why\ntests/keys/\n%s\n' "$1" > "$repo/.secret-scan-allow"
	gitc add -A; gitc commit -q -m "a list"
	local on_base; on_base="$(git -C "$repo" rev-parse HEAD)"
	printf 'AWS_KEY=AKI%s\n' "A$(rep Q 16)" > "$repo/c.txt"; gitc add -A; gitc commit -q -m "a credential"
	out="$( (cd "$repo" && "$SCRIPT" "$on_base" HEAD 2>&1) )"; rc=$?
}
plain_line="docs/keys/"
listed_case "$plain_line"
check "a list of plain paths leaves the credential to be found" "1 1" "$rc $(printf '%s' "$out" | grep -c 'c.txt:1 AWS access key')"
check "and nothing is said about the list" 0 "$(printf '%s' "$out" | grep -c 'secret-scan-allow line')"
# shellcheck disable=SC1003 # the quote after the backslash closes the string; the backslash is the character listed
for ch in '*' '?' '[' '^' '$' '|' '(' '\'; do
	listed_case "docs/k${ch}eys/"
	check "a line with $ch stops the scan" 1 "$rc"
	check "and the message names its line and the character" 1 "$(printf '%s' "$out" | grep -cF "::error::.secret-scan-allow line 3 holds $ch, and a path is listed as it is")"
	check "and no clean verdict is given" 0 "$(printf '%s' "$out" | grep -c 'no credential-shaped lines added')"
done
for pattern in '.*' '^.*$' '^[a-y].*' '.*[^a]$' '[unclosed' 'a|' '(|x)' '.*/.*'; do
	listed_case "$pattern"
	check "the expression '$pattern' is refused, not read" 1 "$rc"
	check "and the message names its line" 1 "$(printf '%s' "$out" | grep -c '::error::.secret-scan-allow line 3 ')"
done
listed_case "/"
check "a lone slash stops the scan" 1 "$rc"
check "and the message says it is only a slash" 1 "$(printf '%s' "$out" | grep -c '::error::.secret-scan-allow line 3 is only a slash, which would exempt every file')"
for rooted in '/docs/keys/' './docs/keys/' '/etc/'; do
	listed_case "$rooted"
	check "the line '$rooted' stops the scan" 1 "$rc"
	check "and the message says paths are relative to the root" 1 "$(printf '%s' "$out" | grep -c '::error::.secret-scan-allow line 3 starts with a slash or ./, and a path is relative to the root of the repository')"
done
for up in '../docs/' 'docs/../x/' 'keys..pem' '..'; do
	listed_case "$up"
	check "the line '$up' stops the scan" 1 "$rc"
	check "and the message says it holds .." 1 "$(printf '%s' "$out" | grep -c '::error::.secret-scan-allow line 3 holds \.\., and a path stays where it is written')"
done
listed_case "docs/keys/a.b-c_d/"
check "dots, dashes and underscores are plain path characters" "1 1" "$rc $(printf '%s' "$out" | grep -c 'c.txt:1 AWS access key')"
listed_case "c.txt"
check "a file entry on the third line is read, and skips the file" "0 1" "$rc $(printf '%s' "$out" | grep -c 'no credential-shaped lines added')"
git -C "$repo" reset -q --hard "$base"

# A range that cannot be taken is a failure, not a clean scan.
out="$( (cd "$repo" && "$SCRIPT" 0000000000000000000000000000000000000000 HEAD 2>&1) )"; rc=$?
check "an unreadable range fails" 1 "$rc"
check "and says nothing was scanned" 1 "$(printf '%s' "$out" | grep -c 'so nothing was scanned')"
check "and gives no clean verdict" 0 "$(printf '%s' "$out" | grep -c 'no credential-shaped lines added')"

"$SCRIPT" >/dev/null 2>&1; check "no arguments fails" 1 "$?"

echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
