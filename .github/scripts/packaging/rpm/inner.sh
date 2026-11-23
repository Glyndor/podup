#!/usr/bin/env bash
#
# Runs INSIDE the rpm builder container. Writes the spec from the template and
# the staged tree, runs rpmbuild once, and copies the one package to /out.
#
# Reads: /stage (read only: root/ and files.txt), /work/podup.spec.in
# Env:   VERSION, RPM_ARCH (x86_64|aarch64), SOURCE_DATE_EPOCH
set -euo pipefail

: "${VERSION:?}" "${RPM_ARCH:?}" "${SOURCE_DATE_EPOCH:?}"

top=/tmp/rpmbuild
rm -rf "$top"
mkdir -p "$top"/{BUILD,RPMS,SRPMS,SOURCES,SPECS}

# %files: every staged path, with the mode it has in the stage so rpm does not
# take it from a file system that may have been mounted with another umask.
# The directories only podup uses are owned by the package, so removing it
# removes them; the ones shared with other packages (shell completion
# directories, the man page directories) belong to those.
files=""
dirs=""
while read -r mode path _sha _size; do
	files+="%attr(${mode}, root, root) /${path}"$'\n'
	case "$path" in
		usr/share/doc/podup/* | usr/share/licenses/podup/*)
			dir="${path%/*}"
			case "$dirs" in *"%dir %attr(0755, root, root) /${dir}"$'\n'*) ;; *) dirs+="%dir %attr(0755, root, root) /${dir}"$'\n' ;; esac
			;;
	esac
done </stage/files.txt
files="${dirs}${files}"

awk -v v="$VERSION" -v f="$files" '
	{ gsub(/@VERSION@/, v); gsub(/@STAGE@/, "/stage") }
	/^@FILES@$/ { printf "%s", f; next }
	{ print }
' /work/podup.spec.in >"$top/SPECS/podup.spec"

# _buildhost: rpm writes the build machine's name into the header, which would
# make the same input produce different bytes on different runners.
rpmbuild -bb \
	--target "${RPM_ARCH}-linux" \
	--define "_topdir $top" \
	--define "_buildhost glyndor" \
	--define "_rpmformat 4" \
	"$top/SPECS/podup.spec" >/tmp/rpmbuild.log 2>&1 || { cat /tmp/rpmbuild.log >&2; exit 1; }

built=("$top"/RPMS/"$RPM_ARCH"/*.rpm)
if [ "${#built[@]}" != 1 ] || [ ! -f "${built[0]}" ]; then
	echo "rpmbuild did not produce exactly one package" >&2
	exit 1
fi
cp "${built[0]}" /out/
chmod 0644 "/out/$(basename "${built[0]}")"
