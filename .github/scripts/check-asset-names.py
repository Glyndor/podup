#!/usr/bin/env python3
"""Assert install.sh and release.yml agree on release asset names.

Releases are immutable: once a tag is published its asset names can never
change. install.sh downloads `podup-${OS}-${ARCH}` for the platform it runs
on, and release.yml publishes a fixed list of assets. If the two drift, the
installer breaks for users and the only remedy is a brand-new patch release.

This check derives every asset name install.sh can request from its platform
case arms and asserts each one is published by release.yml. It runs in CI so
the drift is caught on the pull request, before any tag exists.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
INSTALL_SH = ROOT / "install.sh"
RELEASE_YML = ROOT / ".github" / "workflows" / "release.yml"


def installer_requestable() -> set[str]:
	"""Asset names install.sh can ask for: podup-<os>-<arch> over all arms."""
	text = INSTALL_SH.read_text()
	template = re.search(r'ARTIFACT="([^"]+)"', text)
	if not template or template.group(1) != "podup-${OS}-${ARCH}":
		sys.exit(
			"check-asset-names: install.sh ARTIFACT template changed; "
			"update this check to match."
		)
	oses = set(re.findall(r'OS="([a-z0-9_]+)"', text))
	arches = set(re.findall(r'ARCH="([a-z0-9_]+)"', text))
	if not oses or not arches:
		sys.exit("check-asset-names: could not parse OS/ARCH arms from install.sh.")
	return {f"podup-{os}-{arch}" for os in oses for arch in arches}


def release_published() -> set[str]:
	"""Asset names declared in the release.yml build matrix."""
	text = RELEASE_YML.read_text()
	assets = set(re.findall(r"^\s*-?\s*asset:\s*(\S+)\s*$", text, re.MULTILINE))
	if not assets:
		sys.exit("check-asset-names: could not parse asset names from release.yml.")
	return assets


def main() -> None:
	requestable = installer_requestable()
	published = release_published()
	missing = sorted(requestable - published)
	if missing:
		lines = "\n".join(f"  - {name}" for name in missing)
		sys.exit(
			"check-asset-names: install.sh can request assets that release.yml "
			f"does not publish:\n{lines}\n"
			f"published: {sorted(published)}"
		)
	print(
		f"OK: all {len(requestable)} installer-requestable assets are published "
		f"by release.yml."
	)


if __name__ == "__main__":
	main()
