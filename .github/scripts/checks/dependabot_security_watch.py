#!/usr/bin/env python3
"""Report Dependabot security pull requests stuck against `main`.

Dependabot security updates do not honour `target-branch`; they open against
the default branch, which here is `main`. `main-guard` refuses any pull
request into `main` whose source is not `develop`, so a security bump lands
with a red check and then sits open. The policy is to keep
`target-branch: develop` and `main-guard` exactly as they are,
re-land every security bump by hand off
`develop`, and make the stuck state loud instead of silent. This script is
the loud part.

Usage:
	gh pr list --repo R --base main --state open \\
		--json number,title,author,headRefName,createdAt \\
		| python3 -I .github/scripts/checks/dependabot_security_watch.py
	python3 -I .github/scripts/checks/dependabot_security_watch.py <file.json>

Input is the JSON list that `gh pr list` prints, read from stdin or from the
file argument. A pull request counts as stuck when Dependabot authored it and
its head is not `develop`. A Dependabot pull request whose head IS `develop`
is a release pull request someone opened on its behalf, not a stuck one.

The author login arrives in two shapes depending on the gh version: the older
user-shaped `dependabot[bot]` and the newer app-shaped `app/dependabot`. Both
are accepted; a login that matches neither is not Dependabot.

Exit codes:
	0  nothing stuck
	1  at least one stuck pull request; each is listed with its number, title
	   and age in days, followed by the re-land procedure
	2  the input could not be read or is not a list of pull requests
"""

import argparse
import json
import sys
from datetime import datetime, timezone

DEPENDABOT_LOGINS = frozenset({"dependabot[bot]", "app/dependabot"})
RELEASE_BRANCH = "develop"

# The procedure is deliberately short and mechanical: a stuck security bump is
# an open advisory, and whoever reads this at 3 a.m. should not have to work
# out the branch flow from the workflow standard.
RELAND_PROCEDURE = """\
Re-land each one by hand off develop; main-guard is right to refuse it as is:
  1. git switch -c deps/<name>-<version> origin/develop
  2. apply the same bump the stuck pull request carries and commit it (-s -S)
  3. open a pull request into develop; it ships to main with the next release
  4. close the stuck pull request with a comment pointing at the new one
"""


def _author_login(pull_request: dict) -> str:
	"""Return the author login, accepting both shapes gh has printed."""
	author = pull_request.get("author")
	if isinstance(author, dict):
		return str(author.get("login", ""))
	if isinstance(author, str):
		return author
	return ""


def _age_days(created_at: object, now: datetime) -> str:
	"""Whole days between `created_at` and `now`, or `?` when unreadable.

	An unreadable date does not drop the pull request from the report; the
	number is what makes the report actionable, the age is decoration.
	"""
	if not isinstance(created_at, str) or not created_at:
		return "?"
	try:
		created = datetime.fromisoformat(created_at.replace("Z", "+00:00"))
	except ValueError:
		return "?"
	if created.tzinfo is None:
		created = created.replace(tzinfo=timezone.utc)
	return str(max((now - created).days, 0))


def is_stuck(pull_request: dict) -> bool:
	"""True when Dependabot authored it and its head is not the release branch."""
	if _author_login(pull_request) not in DEPENDABOT_LOGINS:
		return False
	return pull_request.get("headRefName") != RELEASE_BRANCH


def stuck_pull_requests(pull_requests: list) -> list:
	"""Filter to the stuck ones, in the order the input listed them.

	Raises `ValueError` on any entry that is not a pull request object with
	a number, so a half-parsed listing fails instead of reporting over a
	smaller set than it claims.
	"""
	stuck = []
	for entry in pull_requests:
		if not isinstance(entry, dict) or not isinstance(entry.get("number"), int):
			raise ValueError("every entry must be an object with an integer number")
		if is_stuck(entry):
			stuck.append(entry)
	return stuck


def load(source) -> list:
	"""Parse the listing from an open text stream; raise ValueError if not a list."""
	parsed = json.load(source)
	if not isinstance(parsed, list):
		raise ValueError("input must be a JSON list of pull requests")
	return parsed


def report(stuck: list, now: datetime, out) -> int:
	"""Print the verdict and return the exit code."""
	if not stuck:
		print("No Dependabot pull request is stuck against main.", file=out)
		return 0
	print(
		f"{len(stuck)} Dependabot pull request(s) open against main that main-guard will never let in:",
		file=out,
	)
	for pull_request in stuck:
		number = pull_request["number"]
		title = pull_request.get("title", "")
		age = _age_days(pull_request.get("createdAt"), now)
		print(f"::error::#{number} stuck against main for {age} days: {title}", file=out)
		print(f"  #{number}  {age:>3} days  {title}", file=out)
	print(file=out)
	print(RELAND_PROCEDURE, end="", file=out)
	return 1


def _parse_args(argv: list) -> argparse.Namespace:
	parser = argparse.ArgumentParser(
		description="Report Dependabot security pull requests stuck against main.",
	)
	parser.add_argument(
		"listing",
		nargs="?",
		help="JSON file as printed by gh pr list; stdin when omitted",
	)
	parser.add_argument(
		"--now",
		help="ISO 8601 instant to compute ages against; defaults to the current UTC time",
	)
	return parser.parse_args(argv)


def main(argv: list) -> int:
	args = _parse_args(argv)
	if args.now:
		try:
			now = datetime.fromisoformat(args.now.replace("Z", "+00:00"))
		except ValueError as exc:
			print(f"--now is not an ISO 8601 instant: {exc}", file=sys.stderr)
			return 2
		if now.tzinfo is None:
			now = now.replace(tzinfo=timezone.utc)
	else:
		now = datetime.now(timezone.utc)

	try:
		if args.listing:
			with open(args.listing, encoding="utf-8") as source:
				pull_requests = load(source)
		else:
			pull_requests = load(sys.stdin)
		stuck = stuck_pull_requests(pull_requests)
	except (OSError, ValueError) as exc:
		# json.JSONDecodeError is a ValueError. Unreadable input is a
		# failure of the watcher, never a clean bill of health.
		print(f"could not read the pull request listing: {exc}", file=sys.stderr)
		return 2

	return report(stuck, now, sys.stdout)


if __name__ == "__main__":
	sys.exit(main(sys.argv[1:]))
