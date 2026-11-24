#!/usr/bin/env bash
#
# Usage: check-artifact-dir.sh DIR sign ASSET
#        check-artifact-dir.sh DIR attest
#        check-artifact-dir.sh DIR exact VERSION SET
#        check-artifact-dir.sh DIR gather DEST
#
# Fails unless DIR holds exactly what the next step expects and nothing else:
# plain files at the top level, no directories, no links, no other names. A job
# that builds produced these files, so a path in one of them is not trusted to
# stay where it was put; this runs from the checkout, before anything signs.
#
#   sign ASSET   the one file named ASSET.
#   attest       binaries and packages, nothing else.
#   exact V SET  exactly the names of the set SET of expected-assets.py for the
#                version V: none missing, none extra. The version is the one the
#                tag was verified to carry.
#   gather DEST  DIR holds one directory per downloaded artifact, each with
#                plain files only; they are copied flat into DEST, and a name
#                carried by two artifacts is an error, where merging them on
#                download would let one replace the other unseen.
set -euo pipefail

dir="${1:?directory}"
mode="${2:?sign, attest, exact or gather}"

[ -d "$dir" ] || { echo "::error::$dir is not a directory" >&2; exit 1; }

if [ "$mode" = gather ]; then
	dest="${3:?destination}"
	mkdir -p "$dest"
	stray="$(find "$dir" -mindepth 1 -maxdepth 1 ! -type d -print)"
	if [ -n "$stray" ]; then
		echo "::error::something that is not an artifact directory sits beside them:" >&2
		printf '%s\n' "$stray" >&2
		exit 1
	fi
	for artifact in "$dir"/*/; do
		[ -d "$artifact" ] || continue
		if [ -n "$(find "$artifact" -mindepth 1 ! -type f -print -quit)" ] \
			|| [ -n "$(find "$artifact" -mindepth 2 -print -quit)" ]; then
			echo "::error::$artifact holds more than plain files at its top level" >&2
			exit 1
		fi
		for file in "$artifact"*; do
			[ -e "$file" ] || continue
			name="$(basename "$file")"
			if [ -e "$dest/$name" ]; then
				echo "::error::two artifacts carry a file named '$name'" >&2
				exit 1
			fi
			cp "$file" "$dest/$name"
		done
	done
	echo "artifacts gathered into $dest."
	exit 0
fi

odd="$(find "$dir" -mindepth 1 ! -type f -print)"
if [ -n "$odd" ]; then
	echo "::error::the artifacts contain something that is not a plain file:" >&2
	printf '%s\n' "$odd" >&2
	exit 1
fi
if find "$dir" -mindepth 2 -type f -print -quit | grep -q .; then
	echo "::error::the artifacts contain a file below the top level:" >&2
	find "$dir" -mindepth 2 -type f -print >&2
	exit 1
fi

names="$(find "$dir" -mindepth 1 -maxdepth 1 -type f -printf '%f\n' | LC_ALL=C sort)"

case "$mode" in
	sign)
		asset="${3:?asset name}"
		if [ "$names" != "$asset" ]; then
			echo "::error::expected exactly '$asset', found:" >&2
			printf '%s\n' "$names" >&2
			exit 1
		fi
		;;
	attest)
		binary='podup-(linux-x86_64|linux-arm64|darwin-arm64|darwin-x86_64|windows-x86_64\.exe|windows-arm64\.exe)'
		version='[0-9]+\.[0-9]+\.[0-9]+'
		package="podup_${version}_(amd64|arm64)\\.deb|podup-${version}-1\\.glyndor\\.any\\.(x86_64|aarch64)\\.rpm|podup-${version}-1-x86_64\\.pkg\\.tar\\.zst"
		stray="$(printf '%s\n' "$names" | grep -Ev "^(${binary}|${package})\$" || true)"
		if [ -n "$stray" ]; then
			echo "::error::the artifacts to attest contain other names:" >&2
			printf '%s\n' "$stray" >&2
			exit 1
		fi
		;;
	exact)
		version="${3:?version}"
		set_name="${4:?set}"
		here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
		wanted="$(python3 -I "$here/expected-assets.py" "$version" "$set_name")" || exit 1
		if [ "$names" != "$wanted" ]; then
			echo "::error::the files are not the set '$set_name' of version $version; the left side is what is here, the right side what is expected:" >&2
			diff <(printf '%s\n' "$names") <(printf '%s\n' "$wanted") >&2 || true
			exit 1
		fi
		;;
	*)
		echo "unknown mode: $mode" >&2
		exit 2
		;;
esac
echo "artifacts in $dir are what $mode expects."
