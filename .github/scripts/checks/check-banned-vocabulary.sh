#!/usr/bin/env bash
# Fail when a tracked file uses a word the style rules ask to leave out.
#
# Those rules list the vocabulary that reads as machine written and the
# promotional keywords that read as advertising. History cannot be cleaned
# (main and develop take no force push), so the discipline has to be at
# writing time, and a rule that lives in prose is mostly followed. This reads what ships: comments, docs, templates,
# workflows and the messages the code prints.
#
# What it matches:
#   - the whole word, ignoring case: the words in WORDS below;
#   - the connective openers Additionally, Furthermore and Moreover, written
#     with a capital letter, because that is how an opener is written and
#     `additionally stashed` in the middle of a sentence is ordinary;
#   - the phrases In conclusion and It's worth noting, ignoring case;
#   - the rocket emoji.
#
# A legitimate use is listed in .github/vocabulary-exceptions.txt as
# path<TAB>text on the line<TAB>reason. It is matched by the text on the line, so
# it survives the line moving, and an entry that no longer matches anything
# fails, so the list cannot outlive its cause.
#
# CHANGELOG.md records history in its own register and is not read. This
# script and the exceptions file have to contain the words and are not read.
set -euo pipefail
export LC_ALL=C

cd "$(git rev-parse --show-toplevel)"
exceptions=.github/vocabulary-exceptions.txt

WORDS='delve|leverage|leverages|leveraged|leveraging|seamless|seamlessly|robust|robustness|comprehensive|comprehensively|cutting-edge|boasts|blazing fast|revolutionary'
OPENERS='Additionally|Furthermore|Moreover'
PHRASES="in conclusion|it['’]s worth noting|the best([^-[:alnum:]]|\$)"
scope=(-- . ':!CHANGELOG.md' ':!Cargo.lock' ':!.github/scripts/checks/check-banned-vocabulary.sh' ":!$exceptions")

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
{
	git grep -n -I -i -w -E "$WORDS" "${scope[@]}" || true
	git grep -n -I -w -E "$OPENERS" "${scope[@]}" || true
	git grep -n -I -i -E "$PHRASES" "${scope[@]}" || true
	git grep -n -I -F '🚀' "${scope[@]}" || true
} | LC_ALL=C sort -u >"$tmp/hits"

declare -A used=()
declare -a paths=() texts=()
if [ -e "$exceptions" ]; then
	number=0
	while IFS= read -r line || [ -n "$line" ]; do
		number=$((number + 1))
		case "$line" in '' | \#*) continue ;; esac
		IFS=$'\t' read -r path text reason extra <<<"$line"
		if [ -z "${path:-}" ] || [ -z "${text:-}" ] || [ -z "${reason// /}" ] || [ -n "${extra:-}" ]; then
			echo "banned-vocabulary: $exceptions:$number: expected path<TAB>text<TAB>reason" >&2
			exit 1
		fi
		paths+=("$path")
		texts+=("$text")
	done <"$exceptions"
fi

failed=0
while IFS= read -r hit; do
	[ -n "$hit" ] || continue
	file="${hit%%:*}"
	rest="${hit#*:}"
	lineno="${rest%%:*}"
	content="${rest#*:}"
	excused=0
	for i in "${!paths[@]}"; do
		if [ "${paths[$i]}" = "$file" ] && [[ "$content" == *"${texts[$i]}"* ]]; then
			used[$i]=1
			excused=1
		fi
	done
	if [ "$excused" -eq 0 ]; then
		echo "::error file=${file},line=${lineno}::vetoed word or phrase (style rules); say it plainly, or list a legitimate use in ${exceptions}"
		echo "${hit}"
		failed=1
	fi
done <"$tmp/hits"

for i in "${!paths[@]}"; do
	if [ -z "${used[$i]:-}" ]; then
		echo "::error file=${exceptions}::the exception for ${paths[$i]} matches nothing; remove it"
		failed=1
	fi
done

if [ "$failed" -ne 0 ]; then exit 1; fi
echo "no vetoed vocabulary in tracked files"
