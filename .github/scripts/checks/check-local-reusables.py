#!/usr/bin/env python3
"""Refuse a path-filtered workflow that does not watch the reusables it calls.

A workflow with a `paths:` filter decides for itself when it runs. A local
copy of a reusable is edited directly: nothing touches the caller, so the
filter does not fire and the reusable merges without having run.

This is the same defect as #1295 one level up, and the reason it is a check
rather than a note in CONTRIBUTING.md is that it is a property of the tree,
so it can be asserted instead of remembered.

No input guards it. `tooling-isolation` takes one because it was written to
be adopted repository by repository; this rule has one consumer, and a switch
that turns a check off is a way for the check to report success without
looking.

Output: writes one `::error file=<path>::...` line per violation to stdout,
exits 0 on clean and 1 on any violation. A workflow that cannot be parsed is
warned and skipped, the same way the other assertions in
reusable-workflow-lint.yml handle it.

Reads: .github/workflows/*.yml, .github/workflows/*.yaml under the working
directory.
"""
import os
import re
import sys

import yaml

sys.path.insert(0, ".github/scripts/lib")
import ci_yaml  # noqa: E402  (the one loader and enumerator; run from the repository root)

# Only a local call is affected. A remote `uses:` carries its own
# SHA, and bumping it edits the caller.
LOCAL_USES_RE = re.compile(
    r"uses:\s*\./(\.github/workflows/[A-Za-z0-9._-]+\.ya?ml)"
)


def triggers(spec):
    # `on:` parses as the boolean True under the YAML 1.1 rules
    # PyYAML follows, so read both spellings rather than assuming
    # which one this file produced.
    on = spec.get("on", spec.get(True))
    return on if isinstance(on, dict) else {}


def main():
    files = ci_yaml.files(".")
    violations = []
    for path in files:
        try:
            with open(path) as fh:
                text = fh.read()
            spec = ci_yaml.load(path)
        except (OSError, yaml.YAMLError) as e:
            print(f"::error file={path}::{e}")
            sys.exit(1)
        if not isinstance(spec, dict):
            continue
        called = sorted(set(LOCAL_USES_RE.findall(text)))
        if not called:
            continue
        for event, cfg in triggers(spec).items():
            if not isinstance(cfg, dict):
                continue
            # Refusing to reason about a filter shape this check
            # does not model is the point. Reporting "no violation"
            # over a filter it could not read would be a pass that
            # never looked.
            if "paths-ignore" in cfg:
                violations.append((
                    path, event,
                    "uses `paths-ignore:`, which this check does not model; "
                    "state the trigger as `paths:` instead",
                ))
                continue
            patterns = cfg.get("paths")
            if not patterns:
                continue
            for reusable in called:
                # A literal entry, or a glob that covers it. Both
                # make the workflow run when the reusable changes,
                # which is the property being asserted.
                if any(
                    p == reusable
                    or re.fullmatch(
                        re.escape(p).replace(r"\*\*", ".*").replace(r"\*", "[^/]*"),
                        reusable,
                    )
                    for p in patterns
                    if isinstance(p, str)
                ):
                    continue
                violations.append((
                    path, event,
                    f"calls `./{reusable}` but its `on.{event}.paths` does not "
                    f"match it, so a pull request that edits only that reusable "
                    f"does not run this workflow",
                ))
    if violations:
        for path, event, msg in violations:
            print(f"::error file={path}::`{os.path.basename(path)}` {msg}")
        sys.exit(1)
    print(
        "local-reusables: every path-filtered workflow matches the reusables it calls."
    )
    sys.exit(0)


if __name__ == "__main__":
    sys.exit(main())
