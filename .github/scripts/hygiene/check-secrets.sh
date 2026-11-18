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
added="$(git diff --no-color -U0 --no-renames "$base...$head" | awk '
	/^\+\+\+ / { file = substr($0, 7); sub(/^b\//, "", file); next }
	/^@@/        { split($3, a, ","); line = substr(a[1], 2) + 0; next }
	/^\+/        { print file ":" line ":" substr($0, 2); line++ }
' | grep -v 'secret-scan:allow' || true)"

# Paths the repository declares as holding throwaway keys on purpose.
allow="$(git show "$head:.secret-scan-allow" 2>/dev/null | grep -vE '^[[:space:]]*(#|$)' || true)"
if [ -n "$allow" ] && [ -n "$added" ]; then
	added="$(printf '%s\n' "$added" | grep -vE -f <(printf '%s\n' "$allow" | sed 's/^/^(/; s/$/)/') || true)"
fi

[ -n "$added" ] || { echo "no credential-shaped lines added"; exit 0; }

patterns=(
	'private key block|:[0-9]+:-----BEGIN ((RSA|EC|OPENSSH|DSA|PGP|ENCRYPTED) )?PRIVATE KEY( BLOCK)?-----[[:space:]]*$'
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
report() { # <name> <matching lines>
	printf '%s\n' "$2" | cut -d: -f1,2 | sed "s|\$| $1|; s|^|::error::|"
	bad=1
}

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
