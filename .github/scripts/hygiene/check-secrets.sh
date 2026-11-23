#!/usr/bin/env bash
#
# Refuse a pull request that adds something shaped like a credential.
#
# Looks only at the lines <base>..<head> adds, so an old file is never a reason to
# fail. It reports file and line and the name of the pattern, never the value: a
# CI log of a public repository is public too.
#
# What it knows: private key blocks (a PEM file, or a PEM with its body inside a
# string; code that only writes or checks the header line is not one), the token formats of AWS, GitHub, GitLab,
# Slack, npm, Google and OpenAI, JWTs, and an assignment of a long value to a name
# that says secret, token, password or key. The last one is what catches a
# credential with a format of its own, such as a signing key in base64. A line that
# carries the word secret-scan:allow is skipped, for a value that is public on
# purpose; say why next to it. A file cannot carry a comment, so a repository that
# keeps throwaway test keys lists the paths in .secret-scan-allow at its root, one
# regular expression per line, each with a comment saying why.
#
# This is a filter for the mistake, not a vault: GitHub's push protection and
# secret scanning remain the other half.
#
# Usage: check-secrets.sh <base> <head>

set -euo pipefail

base="${1:?base}"; head="${2:?head}"

# One "file:line:text" per added line. awk only parses the diff; the matching is
# grep, whose interval expressions behave the same on every runner (mawk, which
# some images ship as awk, does not always honour them).
diff_lines() { # <from> <to>: the lines <to> adds over <from>
	git diff --no-color -U0 --no-renames --text --no-textconv --no-ext-diff "$1" "$2" | awk '
		/^\+\+\+ / { file = substr($0, 7); sub(/^b\//, "", file); next }
		/^@@/        { split($3, a, ","); line = substr(a[1], 2) + 0; next }
		/^\+/        { print file ":" line ":" substr($0, 2); line++ }
	'
}

# What the range adds, read three ways. The net diff is what lands. But a credential
# added by one commit and taken out by the next is in the history all the same, and
# history cannot be taken back, so every commit is read against each of its parents as
# well, and so is its message (a token pasted into a message is a token published).
empty_tree="$(git hash-object -t tree /dev/null)"
added="$({
	diff_lines "$(git merge-base "$base" "$head" 2>/dev/null || echo "$base")" "$head"
	while IFS= read -r sha; do
		[ -n "$sha" ] || continue
		parents="$(git rev-list --parents -n 1 "$sha" | cut -d' ' -f2-)"
		[ -n "$parents" ] || parents="$empty_tree"
		for parent in $parents; do diff_lines "$parent" "$sha"; done
		git show -s --format='%B' "$sha" | awk -v f="commit-${sha:0:12}" '{ print f ":" NR ":" $0 }'
	done < <(git rev-list "$base..$head")
} | awk '!seen[$0]++' | grep -v 'secret-scan:allow' || true)"

# Paths the repository declares as holding throwaway keys on purpose.
allow="$(git show "$head:.secret-scan-allow" 2>/dev/null | grep -vE '^[[:space:]]*(#|$)' || true)"
if [ -n "$allow" ] && [ -n "$added" ]; then
	added="$(printf '%s\n' "$added" | grep -vE -f <(printf '%s\n' "$allow" | sed 's/^/^(/; s/$/)/') || true)"
fi

[ -n "$added" ] || { echo "no credential-shaped lines added"; exit 0; }

patterns=(
	'private key block|:[0-9]+:[[:space:]]*(-[[:space:]]+)?([A-Za-z0-9_."'"'"'-]+[[:space:]]*[:=][[:space:]]*)?([|>][+-]?[[:space:]]*)?("""|'"'"''"'"''"'"'|["'"'"'`])?-----BEGIN ((RSA|EC|OPENSSH|DSA|PGP|ENCRYPTED) )?PRIVATE KEY( BLOCK)?-----[[:space:]]*$'
	'private key inside a string|-----BEGIN ((RSA|EC|OPENSSH|DSA|PGP|ENCRYPTED) )?PRIVATE KEY( BLOCK)?-----(\\n|[[:space:]])*[A-Za-z0-9+/]{20,}'
	'AWS access key|(AKIA|ASIA)[0-9A-Z]{16}'
	'GitHub token|gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{50,}'
	'GitLab token|glpat-[A-Za-z0-9_-]{20}'
	'Slack token|xox[baprs]-[A-Za-z0-9-]{10,}'
	'npm token|npm_[A-Za-z0-9]{36}'
	'Google API key|AIza[0-9A-Za-z_-]{35}'
	'API key|sk-[A-Za-z0-9]{32,}'
	'JWT|eyJ[A-Za-z0-9_-]{15,}\.eyJ[A-Za-z0-9_-]{10,}'
)

bad=0
# A header and the body of the key on the next added line, however each is quoted or
# indented: a key put into a string over several lines has no header at the start of
# a line and no body on the header's line.
header_re='-----BEGIN ((RSA|EC|OPENSSH|DSA|PGP|ENCRYPTED) )?PRIVATE KEY( BLOCK)?-----'
body_re='[[:space:]]*["'"'"'`]?[A-Za-z0-9+/]{32,}={0,2}[[:space:]]*(\\n)?["'"'"'`]?,?[[:space:]]*\\?$'
spread=""
while IFS= read -r hit; do
	[ -n "$hit" ] || continue
	file_line="$(printf '%s' "$hit" | grep -oE '^.*:[0-9]+:' | head -1)"
	prefix="${file_line%:*:}"
	n="${file_line%:}"; n="${n##*:}"
	next="${prefix}:$((n + 1)):"
	if printf '%s\n' "$added" | grep -F -- "$next" | grep -qE -- "^.*:[0-9]+:${body_re}"; then
		spread="${spread:+$spread$'\n'}${prefix}:${n}:spread"
	fi
done < <(printf '%s\n' "$added" | grep -E -- "$header_re" || true)
report() { # <name> <matching lines>
	printf '%s\n' "$2" | cut -d: -f1,2 | sed "s|\$| $1|; s|^|::error::|"
	bad=1
}

[ -z "$spread" ] || report "private key spread over lines" "$spread"

for entry in "${patterns[@]}"; do
	name="${entry%%|*}"; re="${entry#*|}"
	hits="$(printf '%s\n' "$added" | grep -E -- "$re" || true)"
	[ -z "$hits" ] || report "$name" "$hits"
done

# A credential with a format of its own is caught by the name it is assigned to.
generic='(token|secret|passw(or)?d|api[_-]?key|private[_-]?key|signing[_-]?key|access[_-]?key|ed25519[_-]?key|credential)[a-z0-9_]*["'"'"']?[ \t]*[:=][ \t]*["'"'"']?[a-z0-9+/_=-]{32,}'
skip='_(file|path|dir|url|name|env|var)[a-z0-9_]*["'"'"']?[ \t]*[:=]|secrets\.|\$\{|\$\(|example|placeholder|dummy|fake|fixture|changeme|redacted|your|<[a-z_-]+>'
hits="$(printf '%s\n' "$added" | grep -iE -- "$generic" | grep -viE -- "$skip" || true)"
[ -z "$hits" ] || report "long value assigned to a secret-like name" "$hits"

if [ "$bad" -ne 0 ]; then
	echo "Remove the value and rotate it if it was ever real; add secret-scan:allow only for a value that is public on purpose." >&2
	exit 1
fi
echo "no credential-shaped lines added"
