#!/usr/bin/env bash
#
# Refuse a pull request that adds something shaped like a credential.
#
# Looks only at what <base>..<head> adds, so an old file is never a reason to fail:
# the net diff, every commit of the range against each of its parents (a credential
# one commit adds and the next takes out is in the history all the same, and history
# cannot be taken back), and every commit message. It reports file and line and the
# name of the pattern, never the value: a CI log of a public repository is public too.
#
# What it knows: private key blocks (a PEM file, or a PEM with its body inside a
# string, indented, listed or quoted, or with its body on the next line; code that only
# writes or checks the header line is not one), the token formats of AWS, GitHub, GitLab,
# Slack, npm, Google and OpenAI, JWTs, and an assignment of a long value to a name
# that says secret, token, password or key, which includes the names of the two
# release signing keys. The last one is what catches a
# credential with a format of its own, such as a signing key in base64. A line that
# carries the word secret-scan:allow is skipped, for a value that is public on
# purpose; say why next to it. A file cannot carry a comment, so a repository that
# keeps throwaway test keys lists the paths in .secret-scan-allow at its root, one
# path per line, each group with a comment saying why. A line is a literal path
# relative to the root of the repository: it exempts that exact file or, when it
# ends in a slash, every file below that directory. There are no wildcards and no
# regular expressions, so what the list exempts is read off the list. A line with a
# wildcard or a regular expression character, one that starts with a slash or with
# ./, one that holds .., and a lone slash fail the scan, naming the line. A line of the form
# `path :: text` exempts one line of one file instead of the whole file: the lines of that
# file whose text is `text` (without their leading and trailing blanks), and no other. An entry
# of that kind that names a line the file no longer has fails the scan until it is removed, so
# no dead exemption stays behind. The list counts once it is on the base branch: a change
# cannot add its own exceptions.
#
# This is a filter for the mistake, not a vault: GitHub's push protection and
# secret scanning remain the other half.
#
# Usage: check-secrets.sh <base> <head>

set -euo pipefail

base="${1:?base}"; head="${2:?head}"

# Bytes, not characters: a line with a byte that is not valid UTF-8 must not stop a
# pattern from matching what comes after it.
export LC_ALL=C

# Separates the path, the line number and the text of a record. It is a control
# character, which no file name that gets this far can hold, so a name cannot forge a
# record, and the text of a line is only ever matched after the second one.
SEP=$'\037'

names="$(mktemp)"; patchfile="$(mktemp)"; baseblobs="$(mktemp)"; raw="$(mktemp)"
allrecs="$(mktemp)"; marked="$(mktemp)"
trap 'rm -f "$names" "$patchfile" "$baseblobs" "$raw" "$allrecs" "$marked"' EXIT

# The header of a private key. A line of that shape is never exempted by itself: the scan has
# to see it to read the lines below it (see the search for a body further down), and a file that
# holds keys on purpose is exempted whole, with its reason, in the list.
header_re='-----BEGIN ((RSA|EC|OPENSSH|DSA|PGP|ENCRYPTED) )?PRIVATE KEY( BLOCK)?-----'

# Every grep that filters accepts "no line" (status 1) and nothing else: a grep that failed
# for any other reason must stop the scan, not read as a scan that found nothing.
matching() { # <grep options and arguments>: the matching lines on stdout, in any case
	local rc=0
	grep -a "$@" || rc=$?
	[ "$rc" -le 1 ] || exit 2
}

# What the repository declares as holding a throwaway key or a false positive on purpose. The
# list is read from <base>, the branch the change lands on, never from <head>: a change that
# carries its own list would be the one deciding what the scan may skip. An entry is read off
# the list, never compiled: an expression cannot be judged by reading it (one can match every
# path but a canary), and a wildcard can reach further than its author meant.
#   path/to/file        the exact path of a file: all of it is skipped;
#   path/to/dir/        a directory (it ends in a slash): every file below it is skipped;
#   path :: text        one line of one file: the lines of that file whose text is `text`
#                       (compared without their leading and trailing blanks) are skipped, and
#                       every other line of the file is read as usual.
# A line entry has to stay true: when the line is no longer in the file the entry is a dead
# exemption, and the scan fails until the entry is removed (see check_dead_line_entries).
parse_list() { # <list text> <prefix>: fills <prefix>_paths, <prefix>_line_paths, <prefix>_line_texts
	local list="$1" prefix="$2" entry path text n=0 ch
	local -n out_paths="${prefix}_paths" out_lpaths="${prefix}_line_paths" out_ltexts="${prefix}_line_texts"
	out_paths=(); out_lpaths=(); out_ltexts=()
	while IFS= read -r entry || [ -n "$entry" ]; do
		n=$((n + 1))
		[[ "$entry" =~ ^[[:space:]]*(#|$) ]] && continue
		path="$entry"; text=""
		if [[ "$entry" == *" :: "* ]]; then
			path="${entry%% :: *}"; text="${entry#* :: }"
			text="$(trim "$text")"
			[ -n "$text" ] || { echo "::error::.secret-scan-allow line $n names a line to exempt but the text is empty, and an empty text would exempt every blank line" >&2; exit 1; }
			[[ ! "$text" =~ $header_re ]] || { echo "::error::.secret-scan-allow line $n exempts the header of a private key; a file that holds keys on purpose is listed whole, with its reason, and never line by line" >&2; exit 1; }
			case "$path" in */) echo "::error::.secret-scan-allow line $n gives a directory and a line, and a line belongs to a file" >&2; exit 1 ;; esac
		fi
		[ "$path" != / ] || { echo "::error::.secret-scan-allow line $n is only a slash, which would exempt every file" >&2; exit 1; }
		# shellcheck disable=SC1003 # the quote after the backslash closes the string; the backslash is the character listed
		for ch in '*' '?' '[' '^' '$' '|' '(' '\'; do
			case "$path" in *"$ch"*) echo "::error::.secret-scan-allow line $n holds $ch, and a path is listed as it is: no wildcards or regular expressions" >&2; exit 1 ;; esac
		done
		case "$path" in /* | ./*) echo "::error::.secret-scan-allow line $n starts with a slash or ./, and a path is relative to the root of the repository" >&2; exit 1 ;; esac
		case "$path" in *..*) echo "::error::.secret-scan-allow line $n holds .., and a path stays where it is written" >&2; exit 1 ;; esac
		if [ -n "$text" ]; then out_lpaths+=("$path"); out_ltexts+=("$text"); else out_paths+=("$path"); fi
	done <<< "$list"
}

# A line without its leading and trailing blanks (a space, a tab, a carriage return).
trim() { # <text>
	local t="$1"
	t="${t#"${t%%[![:space:]]*}"}"
	printf '%s' "${t%"${t##*[![:space:]]}"}"
}

base_paths=(); base_line_paths=(); base_line_texts=()
# shellcheck disable=SC2034 # filled through a nameref in parse_list
head_paths=(); head_line_paths=(); head_line_texts=()
if git cat-file -e "$base:.secret-scan-allow" 2>/dev/null; then
	list="$(git show "$base:.secret-scan-allow")" # set -e: a list that exists and cannot be read stops the scan
	parse_list "$list" base
fi
# The list as the change leaves it is read too, for what it must not hold: an entry that is
# not valid, and a line entry that is dead. It exempts nothing.
if git cat-file -e "$head:.secret-scan-allow" 2>/dev/null; then
	list="$(git show "$head:.secret-scan-allow")"
	parse_list "$list" head
fi
allowed=("${base_paths[@]+"${base_paths[@]}"}")

# A line entry that is in effect (it is in the list of the base) and is still in the list of the
# change must not name a line that the file had and the change takes away: a dead exemption
# fails the scan until its entry is removed. The entry for a line that the change removes, or
# for a file it deletes, goes in the same change. An entry for a line that the base file never
# had is not dead but waiting (the entry reaches the base first, the line follows), and an entry
# that only the change adds is not in effect yet and is not asked for anything.
file_has_line() { # <commit> <path> <text>: the file at that commit has a line with that text
	local file_text line
	git cat-file -e "$1:$2" 2>/dev/null || return 1
	file_text="$(git show "$1:$2" | tr -d '\000')" # set -e: an unreadable file stops the scan
	while IFS= read -r line || [ -n "$line" ]; do
		[ "$(trim "$line")" != "$3" ] || return 0
	done <<< "$file_text"
	return 1
}
check_dead_line_entries() {
	local i j in_head
	for i in "${!base_line_paths[@]}"; do
		in_head=false
		for j in "${!head_line_paths[@]}"; do
			if [ "${head_line_paths[j]}" = "${base_line_paths[i]}" ] && [ "${head_line_texts[j]}" = "${base_line_texts[i]}" ]; then in_head=true; fi
		done
		[ "$in_head" = true ] || continue
		if file_has_line "$base" "${base_line_paths[i]}" "${base_line_texts[i]}" && ! file_has_line "$head" "${base_line_paths[i]}" "${base_line_texts[i]}"; then
			echo "::error::.secret-scan-allow exempts a line of ${base_line_paths[i]} that is not in the file any more; remove the entry" >&2
			exit 1
		fi
	done
}
check_dead_line_entries

# Whether the list exempts a path: the path is the entry, or the entry is a directory
# (it ends in a slash) and the path lies below it.
is_allowed() { # <path>
	local entry
	for entry in ${allowed[@]+"${allowed[@]}"}; do
		[ "$1" != "$entry" ] || return 0
		case "$entry" in */) [[ "$1" != "$entry"* ]] || return 0 ;; esac
	done
	return 1
}

# The range is listed first and on its own: inside `done < <(...)` the status of git
# rev-list is dropped, the loop would not run and the script would say nothing was added
# over a range it never read.
commits="$(git rev-list "$base..$head")" || { echo "::error::could not list $base..$head; the range could not be checked, so nothing was scanned" >&2; exit 1; }
merge_base="$(git merge-base "$base" "$head")" || { echo "::error::no merge base of $base and $head, so nothing was scanned" >&2; exit 1; }
empty_tree="$(git hash-object -t tree /dev/null)"
# The content the base already holds: a file with one of these blobs adds nothing to the
# history, whatever its path, so a move or a copy of a file is not scanned again.
git ls-tree -r "$base" | awk '{ print $3 }' | sort -u > "$baseblobs"

# One "path<SEP>line<SEP>text" per added line, appended to $added. The files come from git's
# own list, and each one is diffed on its own and reported under the name git gave: a path
# read out of the patch would be read out of its text too, and a line a file adds that begins
# "++ b/" is a "+++ b/" line inside the hunk. A diff that cannot be taken stops the scan:
# reading nothing must never read as finding nothing.
added=""
scan_pair() { # <from> <to>: what <to> adds over <from>, both a tree-ish
	local from="$1" to="$2" file new old unread lines
	git --literal-pathspecs diff --name-only -z "$from" "$to" > "$names" || { echo "::error::could not take the diff $from $to, so nothing was scanned" >&2; exit 1; }
	# A name with a control character (a line break, a tab) is refused whole, before any
	# record is made. Bytes outside ASCII are names like any other.
	[ "$(tr -d '\000\040-\176\200-\377' < "$names" | wc -c)" -eq 0 ] || { echo "::error::a changed file has a control character in its name, so nothing was scanned" >&2; exit 1; }
	while IFS= read -r -d '' file; do
		if is_allowed "$file"; then continue; fi
		new="$(git rev-parse -q --verify "$to:$file" || true)"
		if [ -n "$new" ] && grep -qxF "$new" "$baseblobs"; then continue; fi
		# The tree being judged carries its own .gitattributes and, on a runner, its own
		# configuration: a file marked -diff or binary, or given a text conversion or an
		# external diff, would show no lines. With --text git reads every file as text, a
		# binary one included. A NUL is dropped from what is read, so a file that holds one is
		# still read line by line, and a grep never takes the stream for binary and stops.
		git --literal-pathspecs diff --no-color --text --no-textconv --no-ext-diff -U0 "$from" "$to" -- "$file" > "$raw" || { echo "::error::could not take the diff of a changed file, so nothing was scanned" >&2; exit 1; }
		tr -d '\000' < "$raw" > "$patchfile"
		# If a changed file still shows nothing to read, the scan stops rather than pass it: a
		# diff with no hunk for a file whose content is not empty and is not the one it started
		# from. A mode change, a deletion and an empty file have nothing to read and pass.
		unread=false
		if ! grep -aq '^@@' "$patchfile"; then
			old="$(git rev-parse -q --verify "$from:$file" || true)"
			if [ -n "$new" ] && [ "$new" != "$old" ] && [ "$(git cat-file -s "$new")" -gt 0 ]; then unread=true; fi
		fi
		[ "$unread" = false ] || { echo "::error::git showed no added lines for a changed file that is not empty, so nothing was scanned" >&2; exit 1; }
		# Inside a hunk every line that starts with + is content; a header is only what comes
		# before the first @@, and the name is not read from it.
		# Every added line goes to the file of all the records; the ones that carry the mark
		# `secret-scan:allow` are left out of what is printed and are listed apart, so that the
		# log can say how many were skipped and where.
		lines="$(FILE="$file" SEP="$SEP" ALL="$allrecs" MARKED="$marked" awk '
			BEGIN { all = ENVIRON["ALL"]; marked = ENVIRON["MARKED"] }
			/^@@/            { inhunk = 1; split($3, a, ","); line = substr(a[1], 2) + 0; next }
			inhunk && /^\+/  {
				text = substr($0, 2); record = ENVIRON["FILE"] ENVIRON["SEP"] line ENVIRON["SEP"] text
				print record >> all
				if (index(text, "secret-scan:allow") == 0) print record
				else print ENVIRON["FILE"] ":" line >> marked
				line++
			}
		' "$patchfile")" || exit 1
		[ -z "$lines" ] || added+="$lines"$'\n'
	done < "$names"
}

# The net diff is what lands.
scan_pair "$merge_base" "$head"
# Each commit against each of its parents (the root against the empty tree), and its message:
# a token pasted into a message is a token published.
while IFS= read -r sha; do
	[ -n "$sha" ] || continue
	# The parents of the commit, and nothing else: `%P` is empty for a root commit, which is then
	# read against the empty tree. (`rev-list --parents | cut -f2-` would give the root commit
	# itself as its parent, and an empty diff.)
	parents="$(git show -s --format=%P "$sha")" || exit 1
	[ -n "$parents" ] || parents="$empty_tree"
	for parent in $parents; do scan_pair "$parent" "$sha"; done
	# The mark is for a value that is public on purpose in a file; in the message of a commit there
	# is none, so it skips nothing there.
	messages="$(git show -s --format='%B' "$sha" | tr -d '\000' | FILE="commit-${sha:0:12}" SEP="$SEP" ALL="$allrecs" awk '
		{ record = ENVIRON["FILE"] ENVIRON["SEP"] NR ENVIRON["SEP"] $0; print record >> ENVIRON["ALL"]; print record }')" || exit 1
	[ -z "$messages" ] || added+="$messages"$'\n'
done <<< "$commits"
added="${added%$'\n'}"
added="$(printf '%s\n' "$added" | awk '!seen[$0]++')"
# Every record, whatever skips it, for what has to read a line together with its neighbours.
added_all="$(awk '!seen[$0]++' "$allrecs")"

# What the mark skipped is said in the log, by file and line and never by content, so that it can
# be read there.
if [ -s "$marked" ]; then
	skipped="$(sort -u "$marked")"
	echo "::notice::secret-scan:allow skipped $(printf '%s\n' "$skipped" | wc -l | tr -d ' ') line(s): $(printf '%s\n' "$skipped" | tr '\n' ' ')"
fi

# The lines of the list itself that name a line of a file the change has: such an entry holds, as
# its text, the very line that it exempts, which is shaped like a credential or it would need no
# entry, and the scan would flag the list for it. It is skipped only when that line is in the file
# of the change, where it is read like any other (and exempt only if the base already says so).
# An entry whose line is not in the file yet is read like any other line.
if [ -n "$added" ]; then
	kept=""
	while IFS= read -r record; do
		[ -n "$record" ] || continue
		record_path="${record%%"$SEP"*}"
		if [ "$record_path" = .secret-scan-allow ]; then
			record_text="${record#*"$SEP"}"; record_text="${record_text#*"$SEP"}"
			if [[ "$record_text" == *" :: "* ]]; then
				entry_path="${record_text%% :: *}"; entry_text="$(trim "${record_text#* :: }")"
				if [ -n "$entry_text" ] && file_has_line "$head" "$entry_path" "$entry_text"; then continue; fi
			fi
		fi
		kept+="$record"$'\n'
	done <<< "$added"
	added="${kept%$'\n'}"
fi

# The lines that the list exempts one by one: same path, same text. Only those lines go; every
# other line of the file stays in.
if [ "${#base_line_paths[@]}" -gt 0 ] && [ -n "$added" ]; then
	kept=""
	while IFS= read -r record; do
		[ -n "$record" ] || continue
		record_path="${record%%"$SEP"*}"; record_text="${record#*"$SEP"}"; record_text="$(trim "${record_text#*"$SEP"}")"
		exempt=false
		for i in "${!base_line_paths[@]}"; do
			if [ "$record_path" = "${base_line_paths[i]}" ] && [ "$record_text" = "${base_line_texts[i]}" ]; then exempt=true; break; fi
		done
		[ "$exempt" = true ] || kept+="$record"$'\n'
	done <<< "$added"
	added="${kept%$'\n'}"
fi

[ -n "$added_all" ] || { echo "no credential-shaped lines added"; exit 0; }

# What follows matches the text of a record, after its path and its line number.
prefix="^[^${SEP}]*${SEP}[0-9]+${SEP}"

patterns=(
	'private key block|^[[:space:]]*(-[[:space:]]+)?([A-Za-z0-9_."'"'"'-]+[[:space:]]*[:=][[:space:]]*)?([|>][+-]?[[:space:]]*)?("""|'"'"''"'"''"'"'|["'"'"'`])?-----BEGIN ((RSA|EC|OPENSSH|DSA|PGP|ENCRYPTED) )?PRIVATE KEY( BLOCK)?-----[[:space:]]*$'
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
	printf '%s\n' "$2" | awk -F"$SEP" -v what="$1" '{ print "::error::" $1 ":" $2 " " what }'
	bad=1
}

for entry in "${patterns[@]}"; do
	name="${entry%%|*}"; re="${entry#*|}"
	case "$re" in
		^*) re="$prefix${re#^}" ;;
		*) re="$prefix.*($re)" ;;
	esac
	hits="$(printf '%s\n' "$added" | matching -E -- "$re")"
	[ -z "$hits" ] || report "$name" "$hits"
done

# A header with the body of the key on the next added line, however each is quoted or
# indented: a key put into a string over several lines has no header that ends its line
# and no body on the header's line. The next line is looked up by path and number, as
# fields, so a path cannot stand for another.
body_re='^[[:space:]]*["'"'"'`]?[A-Za-z0-9+/]{32,}={0,2}[[:space:]]*(\\n)?["'"'"'`]?,?[[:space:]]*\\?$'
spread=""
# The candidates are captured first: a substitution inside a here-string would hide the status
# of `matching` from set -e, and a grep that failed would read as a scan with nothing to look at.
candidates="$(printf '%s\n' "$added_all" | matching -E -- "$prefix.*($header_re)")" || exit 2
while IFS="$SEP" read -r path n text; do
	[ -n "$path" ] || continue
	# Every record of that path and number, not the first: the same pair turns up once per
	# commit that touches the line, and the body can be in any of them.
	recs="$(printf '%s\n' "$added_all" | P="$path" N="$((n + 1))" awk -F"$SEP" '$1 == ENVIRON["P"] && $2 == ENVIRON["N"] { print }')" || exit 1
	while IFS= read -r rec; do
		[ -n "$rec" ] || continue
		next="${rec#*"$SEP"}"; next="${next#*"$SEP"}"
		if printf '%s\n' "$next" | grep -aqE -- "$body_re"; then
			spread="${spread:+$spread$'\n'}${path}${SEP}${n}${SEP}spread"
			break
		fi
	done <<< "$recs"
done <<< "$candidates"
[ -z "$spread" ] || report "private key spread over lines" "$spread"

# A credential with a format of its own is caught by the name it is assigned to.
generic_name='(token|secret|passw(or)?d|api[_-]?key|private[_-]?key|signing[_-]?key|access[_-]?key|ed25519[_-]?key|mldsa[0-9_-]*key|credential)[a-z0-9_]*'
# A blank is a space, a tab, or the two characters backslash and t, which is how a tab is
# written in the text of a JSON string or of generated YAML. A quote may carry a backslash in
# front of it (`token=\"...\"`) for the same reason.
generic_blank='(\\t|[[:blank:]])*'
generic_quote='(\\)?["'"'"']?'
# An optional type between the name and the operator (`const TOKEN: &str = "..."`,
# `token: string = "..."`, `let k: Vec<u8> = ...`, `token: Arc<Mutex<Option<Vec<u8>>>> = ...`).
# A type never crosses into the next assignment: a comma or a semicolon is part of it only
# inside a bracket group, and a group holds no `=`, no quote and no other character that starts
# a value. Brackets are not balanced and their depth is not counted, so a type nests as deep as
# it likes (`Arc<Mutex<Option<Vec<u8>>>>`, `ClassVar[Optional[List[str]]]`): a group opens with
# `<` or `[`, runs over letters, blanks, commas, semicolons, colons and brackets, and closes with
# `>` or `]`. The first character of a bracket expression is the `]`, so that it is read as a
# character of the class.
generic_type_inner='[][<>A-Za-z0-9_&'"'"',;:[:blank:]]'
generic_type_group='[<[]'"$generic_type_inner"'*[]>]'
generic_annotation='('"$generic_blank"':'"$generic_blank"'[A-Za-z_&]([A-Za-z0-9_&'"'"']|[[:blank:]]|'"$generic_type_group"')*)?'
# The operator is one or more characters (`=`, `:`, `:=`, `=>`, `==`), then the value. The name
# is a group of its own, the first, and the value the last.
generic_operator='(:=|=>|==|=|:)'
generic_value='([a-z0-9+/_=-]{32,})'
generic="(${generic_name})${generic_quote}${generic_annotation}${generic_blank}${generic_operator}${generic_blank}${generic_quote}${generic_value}"
# The same with no type, read on its own as well: a match that took a type must not hide a
# plain assignment that it overlaps.
generic_plain="(${generic_name})${generic_quote}${generic_blank}${generic_operator}${generic_blank}${generic_quote}${generic_value}"
# What excuses an assignment is read in the assignment itself, not in the line around it:
# the name of a variable that holds a file, a path or an address is not a secret (the name,
# and only the name: not the type, not a neighbour), and a value that carries a sample word of
# its own, as a word and not as a run of letters of a key, is a sample.
named_path='_(file|path|dir|url|name|env|var)[a-z0-9_]*$'
sample_word='(^|[^a-z0-9])(example|placeholder|dummy|fake|fixture|changeme|redacted|your)([^a-z0-9]|$)'
assignment_is_real() { # <regex> <text>: some assignment the regex finds in the text is not excused
	local rest="$2" whole name value before
	while [[ "$rest" =~ $1 ]]; do
		whole="${BASH_REMATCH[0]}"
		name="${BASH_REMATCH[1]}"
		value="${BASH_REMATCH[${#BASH_REMATCH[@]} - 1]}"
		if [[ ! "$name" =~ $named_path ]] && [[ ! "$value" =~ $sample_word ]]; then return 0; fi
		# Past the name only, so that what follows on the line is read too, and not past the whole
		# match: an excuse for one assignment cannot reach into the next.
		before="${rest%%"$whole"*}"
		rest="${rest:$((${#before} + ${#name}))}"
	done
	return 1
}
real_value() { # <text of a line>: succeeds when some assignment in it is not a sample
	local found=false
	shopt -s nocasematch
	if [[ "$1" =~ $generic ]] || [[ "$1" =~ $generic_plain ]]; then found=true; fi
	if [ "$found" = true ] && { assignment_is_real "$generic" "$1" || assignment_is_real "$generic_plain" "$1"; }; then
		shopt -u nocasematch
		return 0
	fi
	shopt -u nocasematch
	# A line the candidate search took and this one does not read is not excused.
	[ "$found" = true ] || return 0
	return 1
}
hits=""
candidates="$(printf '%s\n' "$added" | matching -iE -- "$prefix.*($generic)")" || exit 2
while IFS= read -r record; do
	[ -n "$record" ] || continue
	text="${record#*"$SEP"}"; text="${text#*"$SEP"}"
	if real_value "$text"; then hits+="$record"$'\n'; fi
done <<< "$candidates"
hits="${hits%$'\n'}"
[ -z "$hits" ] || report "long value assigned to a secret-like name" "$hits"

if [ "$bad" -ne 0 ]; then
	echo "Remove the value and rotate it if it was ever real; add secret-scan:allow only for a value that is public on purpose." >&2
	exit 1
fi
echo "no credential-shaped lines added"
