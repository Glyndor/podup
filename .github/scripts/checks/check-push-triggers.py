#!/usr/bin/env python3
"""No workflow repeats on a push to `develop` what the merge queue already ran.

The queue runs a workflow on the commit that lands, and the push to `develop` that
follows carries the same commit, so a workflow that has both triggers runs twice on one
tree. Three rules, from the loaded YAML of every workflow:

  1. A workflow with a `merge_group` trigger has no push to `develop`, with no exception.
  2. A workflow with a push that reaches `develop` is on the list below, with its reason.
     Those are workflows the queue does not run (they are filtered by path, or they exist
     to prove that something is alive on every merge), so the push is the only run on the
     commit that lands and not a second one.
  3. Every entry of the list is a workflow that still exists and still has that push, so
     the list cannot keep a reason for something that is gone.

Usage: check-push-triggers.py [<root>]   (exit 1 on any finding)
"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
import ci_yaml  # noqa: E402

# workflow file -> why it still runs on a push to develop
ALLOWED = {
    "asset-contract.yml": "filtered to the installer and release files, and the queue does not run it: the push is its only run on the commit that lands",
    "binary-budget.yml": "filtered to the files the budget reads, and the queue does not run it: the push is its only run on the commit that lands",
    "debian-build.yml": "filtered to the packaging inputs, and the queue does not run it: the push is its only run on the commit that lands",
    "dependabot-security-watch.yml": "proves on every merge that the watcher is alive, which tells a quiet repository from a dead schedule",
    "lint-powershell.yml": "filtered to PowerShell files, and the queue does not run it: the push is its only run on the commit that lands",
    "lint-shell.yml": "filtered to shell files and the checks they hold, and the queue does not run it: the push is its only run on the commit that lands",
    "linux-hardening.yml": "filtered to the build inputs, and the queue does not run it: the push is its only run on the commit that lands",
    "repository-health.yml": "proves on every push to a protected branch that the schedule is alive, which tells a green branch from a dead cron",
    "supply-chain.yml": "filtered to the dependency files, and the queue does not run it: the push is its only run on the commit that lands",
    "windows-arm64.yml": "filtered to the build inputs, and the queue does not run it: the push is its only run on the commit that lands",
}


def triggers(spec):
    on = spec.get("on", spec.get(True))
    if isinstance(on, str):
        return {on: None}
    if isinstance(on, list):
        return {name: None for name in on}
    return on if isinstance(on, dict) else {}


def pushes_to_develop(on):
    if "push" not in on:
        return False
    push = on["push"]
    if not isinstance(push, dict):
        return True  # `push:` alone is every branch
    if "tags" in push and "branches" not in push and "branches-ignore" not in push:
        return False  # tags only
    branches = push.get("branches")
    if branches is not None:
        return "develop" in ([branches] if isinstance(branches, str) else branches)
    ignored = push.get("branches-ignore")
    if ignored is not None:
        return "develop" not in ([ignored] if isinstance(ignored, str) else ignored)
    return True


def findings(root):
    out = []
    pushing = set()
    for path in ci_yaml.workflows_in(os.path.join(root, ci_yaml.WORKFLOW_DIR)):
        name = os.path.basename(path)
        try:
            on = triggers(ci_yaml.load(path) or {})
        except Exception as e:
            out.append(f"{name}: cannot be read ({e})")
            continue
        if not pushes_to_develop(on):
            continue
        pushing.add(name)
        if "merge_group" in on:
            out.append(f"{name}: runs on a push to develop and in the merge queue, so it runs twice on the commit that lands; drop the push")
        elif name not in ALLOWED:
            out.append(f"{name}: runs on a push to develop and is not on the list in check-push-triggers.py; the queue tests that commit, so either drop the push or list it with its reason")
    for name, reason in ALLOWED.items():
        if not reason.strip():
            out.append(f"{name}: is on the list with no reason")
        if name not in pushing:
            out.append(f"{name}: is on the list of push triggers but has no push to develop (stale entry)")
    return out


def main():
    root = sys.argv[1] if len(sys.argv) > 1 else "."
    found = findings(root)
    for f in found:
        print(f"push-triggers: {f}")
    if not found:
        print(f"push-triggers: {len(ALLOWED)} workflow(s) push to develop, each for its reason, and none of them is also in the queue.")
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
