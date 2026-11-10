#!/usr/bin/env bash
#
# Reads the tool versions this repository pins and compares each one with the
# tool's latest upstream release. The pin-watch workflow runs this every
# Monday and turns every difference into a row of the drift issue it files.
#
# Two properties matter more than the list of tools:
#
#   - The pinned version is READ FROM THE FILES, never carried here. A
#     watcher that carries its own copy of the pin reports success over a
#     set it stopped watching, and the day the two disagree it says the pin
#     is fine while CI installs something else.
#   - A pin that cannot be read FAILS THE RUN. A watcher that skips what it
#     cannot parse reports "no drift" over a smaller set than it claims, and
#     the pin it skipped is often the one that drifted.
#
# Output: one TSV row per difference, on stdout:
#
#     NAME <TAB> PATH <TAB> PINNED <TAB> UPSTREAM
#
# The workflow appends the rows to the drift table it already builds.
# Progress and diagnostics go to stderr, so they never reach that table.
#
# The upstream version comes from `gh release view --repo <org/repo>
# --json tagName`, GitHub's porcelain, which reads no API of its own. Every
# tool watched here publishes a release, so no version is read from a
# package registry instead: the number a registry carries and the number a
# release carries are the same one, and the release is the thing a human
# goes and looks at.
set -uo pipefail
export LC_ALL=C

die() {
	echo "::error::$*" >&2
	exit 1
}

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd) || die "cannot find the repository root"
cd "$ROOT" || die "cannot enter $ROOT"

# The version a text carries, whatever else the text says. The leading `v`
# is kept: some tools pin the release tag as it is published and others pin
# the bare version, and the file says which one it holds.
version_of() { # <text>
	printf '%s\n' "$1" | grep -m1 -oE 'v?[0-9]+(\.[0-9]+){1,2}'
}

# A bare version, with or without the patch. The shape every tool here
# publishes its version under.
is_version() { # <value>
	printf '%s\n' "$1" | grep -qE '^[0-9]+(\.[0-9]+){1,2}$'
}

# The default of a reusable workflow's own input, read from the block that
# key opens. The block ends at the first line that is not deeper than the
# key, so an input that lost its default reads as missing instead of
# picking up the next input's default.
read_input_default() { # <file> <key>
	awk -v key="$2" '
	{
		match($0, /^[ \t]*/)
		indent = RLENGTH
		if (!inblock) {
			if ($0 ~ "^[ \t]*" key ":[ \t]*$") {
				inblock = 1
				keyindent = indent
			}
			next
		}
		if ($0 ~ /^[ \t]*$/ || $0 ~ /^[ \t]*#/) next
		if (indent <= keyindent) exit
		if ($0 ~ /^[ \t]*default:[ \t]*/) {
			value = $0
			sub(/^[ \t]*default:[ \t]*/, "", value)
			gsub(/"/, "", value)
			print value
			exit
		}
	}
	' "$1"
}

# The line of a workflow's own environment entry.
read_env_value() { # <file> <key>
	grep -m1 -E "^[[:space:]]*$2:[[:space:]]*\"?v?[0-9]" "$1"
}

# The line that installs a crate at an exact version. The version has to be
# a literal: the reusables install with `--version "$VAR"`, and the pin
# they carry is the input default this reads elsewhere. Words in front of
# `cargo install` (`elif `, `RUST_LOG=x `) are allowed, since the coverage
# legs of the Podman lanes install inside a branch of a script.
read_install_line() { # <file> <crate>
	grep -m1 -E "^[[:space:]]*([^#[:space:]]+[[:space:]]+)*cargo install .*$2([^a-zA-Z0-9_-]|\$).*--version[[:space:]]*\"=" "$1"
}

# The version a pin site holds, or nothing when the site cannot be read.
read_pin() { # <shape> <file> <key>
	local text
	case "$1" in
		input) text=$(read_input_default "$2" "$3") ;;
		env) text=$(read_env_value "$2" "$3") ;;
		install) text=$(read_install_line "$2" "$3") ;;
		*) die "unknown pin shape '$1'" ;;
	esac
	version_of "$text"
}

# The version a release tag carries. The prefix is part of the tag:
# cargo-audit publishes `cargo-audit/v0.22.2` and cargo-cyclonedx publishes
# `cargo-cyclonedx-0.5.9`, while the rest publish a bare or v-prefixed
# version. A tag that does not start with the prefix is not a release of
# that tool's version.
tag_version() { # <tag> <prefix>
	local rest
	case "$1" in
		"$2"*) rest=${1#"$2"} ;;
		*) return 1 ;;
	esac
	rest=${rest#v}
	printf '%s\n' "$rest"
}

# The latest upstream version of a tool, read from its releases.
#
# `gh release view` answers with the repository's newest release, and for
# one of these repositories that newest release is not a version of the
# tool: rustsec/cargo-audit publishes the CVSS schema (`cvss/v3.0.0`)
# alongside its crate releases, and that schema is newer than the crate.
# The release list is consulted when the newest release does not carry a
# version, and the newest release that does is the answer.
upstream_version() { # <repo> <prefix>
	local tag version candidate best
	local tags=""
	tag=$(gh release view --repo "$1" --json tagName --jq .tagName 2>/dev/null) || tag=""
	if [ -n "$tag" ]; then
		version=$(tag_version "$tag" "$2") || version=""
		if is_version "$version"; then
			printf '%s\n' "$version"
			return 0
		fi
	fi
	tags=$(gh release list --repo "$1" --json tagName --limit 30 --jq '.[].tagName' 2>/dev/null) || tags=""
	best=""
	while IFS= read -r tag; do
		[ -n "$tag" ] || continue
		candidate=$(tag_version "$tag" "$2") || continue
		is_version "$candidate" || continue
		if [ -z "$best" ] || [ "$(printf '%s\n%s\n' "$best" "$candidate" | LC_ALL=C sort -V | tail -n1)" = "$candidate" ]; then
			best=$candidate
		fi
	done <<EOF
$tags
EOF
	[ -n "$best" ] || return 1
	printf '%s\n' "$best"
}

emit() { # <name> <path> <pinned> <upstream>
	printf '%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4"
}

# ---- the tools and where their versions come from --------------------------
# TOOL|UPSTREAM REPOSITORY|TAG PREFIX
#
# The prefix is the part of the release tag that is not the version. A tool
# declared here and pinned nowhere below is a hole in the watcher, and the
# run fails saying so; so does a pin whose tool nobody declares.
declare -A repo_of=()
declare -A prefix_of=()
tool_order=()

while IFS='|' read -r tool repo prefix; do
	[ -n "$tool" ] || continue
	tool_order+=("$tool")
	repo_of["$tool"]="$repo"
	prefix_of["$tool"]="$prefix"
done <<'TOOLS'
cargo-llvm-cov|taiki-e/cargo-llvm-cov|v
cargo-semver-checks|obi1kenobi/cargo-semver-checks|v
cargo-audit|rustsec/cargo-audit|cargo-audit/v
cargo-deny|EmbarkStudios/cargo-deny|
cargo-fuzz|rust-fuzz/cargo-fuzz|
cargo-cyclonedx|CycloneDX/cyclonedx-rust-cargo|cargo-cyclonedx-
cargo-about|EmbarkStudios/cargo-about|
shellcheck|koalaman/shellcheck|v
actionlint|rhysd/actionlint|v
PSScriptAnalyzer|PowerShell/PSScriptAnalyzer|
TOOLS

# ---- the sites that pin them -----------------------------------------------
# TOOL|FILE|SHAPE|KEY
#
# SHAPE says how the pin is written: `input` is a reusable workflow's own
# input default, `env` is an environment entry, and `install` is a
# `cargo install` line carrying a literal version. Every site is listed,
# not only the ones that are behind: a site that is not here is a site
# nobody watches, which is how two of these tools drifted in the first
# place.
declare -A sites_of=()
site_count=0

sites_table=$(cat <<'SITES'
cargo-llvm-cov|.github/workflows/reusable-rust-ci.yml|input|llvm-cov-version
cargo-llvm-cov|.github/workflows/podman-lane.yml|install|cargo-llvm-cov
cargo-llvm-cov|.github/workflows/podman-lane-develop-nightly.yml|install|cargo-llvm-cov
cargo-semver-checks|.github/workflows/reusable-rust-ci.yml|input|semver-checks-version
cargo-audit|.github/workflows/reusable-rust-audit.yml|input|cargo-audit-version
cargo-deny|.github/workflows/reusable-rust-audit.yml|input|cargo-deny-version
cargo-fuzz|.github/workflows/reusable-rust-fuzz.yml|input|cargo-fuzz-version
cargo-cyclonedx|.github/workflows/reusable-rust-supply-chain.yml|input|cyclonedx-version
cargo-about|.github/workflows/reusable-rust-supply-chain.yml|input|cargo-about-version
cargo-about|.github/workflows/release.yml|install|cargo-about
cargo-audit|.github/workflows/release.yml|install|cargo-audit
cargo-cyclonedx|.github/workflows/release.yml|install|cargo-cyclonedx
shellcheck|.github/workflows/reusable-shell-ci.yml|input|shellcheck-version
shellcheck|.github/workflows/reusable-workflow-lint.yml|input|shellcheck-version
actionlint|.github/workflows/reusable-workflow-lint.yml|env|ACTIONLINT_VERSION
PSScriptAnalyzer|.github/workflows/reusable-powershell-ci.yml|input|pssa-version
SITES
)
while IFS='|' read -r tool file shape key; do
	[ -n "$tool" ] || continue
	if [ -z "${repo_of[$tool]:-}" ]; then
		die "the $tool pin at $file has no upstream declared in the tool table"
	fi
	if [ ! -f "$file" ]; then
		die "the $tool pin site $file does not exist"
	fi
	pinned=$(read_pin "$shape" "$file" "$key")
	if [ -z "$pinned" ]; then
		die "could not read the $tool pin ($key) from $file"
	fi
	case "$shape" in
		install) label=install ;;
		*) label=$key ;;
	esac
	sites_of["$tool"]="${sites_of[$tool]:-}$file"$'\t'"$label"$'\t'"$pinned"$'\n'
	site_count=$((site_count + 1))
done <<<"$sites_table"

echo "watched ${#tool_order[@]} tools over $site_count pin sites" >&2

# Both tables are checked against each other before anything is compared, so
# a hole in the watcher is reported before a partial table is printed.
for tool in "${tool_order[@]}"; do
	if [ -z "${sites_of[$tool]:-}" ]; then
		die "$tool is declared as watched but no pin site reads it"
	fi
done

# The tables above are carried here, so a pin written in a workflow and never
# added to them would be watched by nobody and reported as no drift. Every pin
# of the shapes below that the workflows hold has to be a site listed above:
# a `*-version` input of a workflow, a `*_VERSION` environment entry with a
# literal version, and a `cargo install` line with a literal `--version "="`.
known_sites=$'\n'$(printf '%s\n' "$sites_table" | cut -d'|' -f2,4)$'\n'
for file in .github/workflows/*.yml; do
	while IFS= read -r key; do
		[ -n "$key" ] || continue
		case "$known_sites" in
			*$'\n'"$file|$key"$'\n'*) ;;
			*) die "$file has the pin $key, which no site above reads: add it to the tables" ;;
		esac
	done < <(grep -oE '^[[:space:]]+[a-z0-9-]+-version:[[:space:]]*$|^[[:space:]]+[A-Z_]+_VERSION:[[:space:]]*"?v?[0-9]' "$file" \
		| sed -E 's/^[[:space:]]+//; s/:.*$//')
	while IFS= read -r line; do
		[ -n "$line" ] || continue
		found=""
		for tool in "${tool_order[@]}"; do
			case "$line" in *"$tool"*) found=1 ;; esac
		done
		[ -n "$found" ] || die "$file installs a tool at an exact version that no tool above names: $line"
	done < <(grep -E '^[[:space:]]*([^#[:space:]]+[[:space:]]+)*cargo install .*--version[[:space:]]*"=' "$file" || true)
done

# ---- the comparison --------------------------------------------------------
for tool in "${tool_order[@]}"; do
	repo="${repo_of[$tool]}"
	prefix="${prefix_of[$tool]}"
	upstream=$(upstream_version "$repo" "$prefix") \
		|| die "could not read the latest release of $repo"
	# The sites of one tool are meant to hold the same version. Nothing
	# derives one site from another, so "the sites disagree" is a drift in
	# its own right and is reported before the upstream comparison, the way
	# the Rust toolchain sites are.
	mapfile -t distinct < <(printf '%s' "${sites_of[$tool]}" | cut -f3 | sed 's/^v//' | LC_ALL=C sort -u)
	if [ "${#distinct[@]}" -gt 1 ]; then
		files=$(printf '%s' "${sites_of[$tool]}" | cut -f1 | LC_ALL=C sort -u | paste -sd, - | sed 's/,/, /g')
		values=$(printf '%s' "${sites_of[$tool]}" | cut -f3 | LC_ALL=C sort -u | paste -sd, - | sed 's/,/, /g')
		emit "$tool (${#distinct[@]} sites disagree)" "$files" "$values" "(they must agree)"
	fi
	while IFS=$'\t' read -r file label pinned; do
		[ -n "$file" ] || continue
		# The upstream is reported in the form the file holds: a file that
		# pins the release tag is told the release tag, not the bare
		# version.
		reported="$upstream"
		case "$pinned" in
			v*) reported="v$upstream" ;;
		esac
		if [ "${pinned#v}" != "$upstream" ]; then
			emit "$tool ($label)" "$file" "$pinned" "$reported"
		fi
	done <<<"${sites_of[$tool]}"
done
