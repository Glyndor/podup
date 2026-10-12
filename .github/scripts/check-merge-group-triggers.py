#!/usr/bin/env python3
"""Refuse a workflow that emits a develop-required check without triggering on
`merge_group`, and refuse any OTHER workflow that carries `merge_group`.

The `protect-develop` ruleset will get a merge queue (squash). In a merge
queue GitHub fires the `merge_group` event with `types: [checks_requested]`
on a temporary branch `gh-readonly-queue/develop/pr-<N>-<sha>`, and required
checks must be reported on that event or the queue never merges. `paths:`
and `branches:` filters do not apply to merge_group, so a workflow that
emits a develop-required check name and only lists `pull_request` will
report nothing in the queue and the gate goes red on its own absence.

The fix is to require every workflow that emits a develop-required check
to also carry `merge_group` in its `on:`. If `merge_group` has a `types:`
key, `checks_requested` must be in it. The companion rule is the other
direction: a workflow that carries `merge_group` but does not emit a
required check would run inside the queue for no reason (release.yml
holds publishing secrets; that is the case this rule exists for, and the
script refuses any other workflow that adds the trigger).

Reusables whose `basename` starts with `reusable-` are scanned only to
learn which required check names they emit; their own `on:` is a
`workflow_call` and not a develop gate. The `on:` trigger check is over
the CALLER files. `reusable-main-guard.yml` is deliberately absent from
the required list: `develop-only` is required on `main` only, and `main`
has no merge queue, so requiring a `merge_group` trigger on its caller
would ask for a path that does not exist.

A workflow that cannot be parsed is warned and skipped, the same way
caller-if and caller-with-defaults do. A scan that read zero emitting
workflows fails closed: an empty answer would be a pass that did not
look.

Output: writes one `::error file=<path>::...` line per violation to
stdout, exits 0 on clean and 1 on any violation (or on an empty scan).
The step body in .github/workflows/reusable-workflow-lint.yml is a
thin shell-out to this script; the extracted step body the shell test
suites run is this script.

Reads: .github/workflows/*.yml, .github/workflows/*.yaml under the
working directory. Symlinks are not followed: a caller escaping the
filter through a symlinked workflow file is out of scope.
"""
import glob
import os
import sys

import yaml

# Reusables whose jobs' `name:` becomes part of a develop-required check
# name when a job in a non-reusable file calls them. The set names the
# files the ruleset matches against: adding a reusable here without
# adding it to the ruleset is a no-op, and a reusable here that the
# ruleset requires will be reported red until its caller gains the
# `merge_group` trigger.
#
# `reusable-main-guard.yml` is intentionally absent: `develop-only` is
# required on `main` only, and `main` does not have a merge queue, so
# its caller (main-guard.yml) does not need (and would not be reached
# with) a `merge_group` trigger. Listing it here would create a
# required-on-paper trigger with no possible caller firing it.
DEVELOP_REQUIRED_REUSABLES = {
    "reusable-rust-ci.yml",
    "reusable-dco.yml",
    "reusable-line-limit.yml",
    "reusable-workflow-lint.yml",
}

# Required check names emitted directly by a job in a non-reusable
# workflow file, not via a reusable call. The set is the value of the
# job's `name:` field that GitHub reports. A future addition lands
# here with its check name.
DIRECT_EMITTER_NAMES = {
    "Supported Podman majors",
}


def triggers(spec):
    """Return the parsed `on:` block.

    `on:` parses as the boolean True under the YAML 1.1 rules PyYAML
    follows, so read both spellings rather than assuming which one this
    file produced. A string or list form is coerced to a single-key
    dict so the rest of the scan can iterate `for event, cfg in
    triggers(spec).items()` uniformly. A non-mapping/non-list/non-str
    value is returned as an empty dict: the scan has nothing to do.
    """
    on = spec.get("on", spec.get(True))
    if isinstance(on, dict):
        return on
    if isinstance(on, str):
        return {on: {}}
    if isinstance(on, list):
        return {item: {} for item in on}
    return {}


def merge_group_cfg(on):
    """Return the `merge_group` sub-block, or None if the trigger is
    absent. The check below cares about three shapes: a bare
    `merge_group:` (treated as no `types:`, so any type is accepted),
    `merge_group: {types: [checks_requested]}` (accepted), and
    `merge_group: {types: [destroyed]}` (refused, no `checks_requested`).
    """
    cfg = on.get("merge_group")
    if cfg is None:
        return None
    if cfg is True:
        return {}
    if isinstance(cfg, dict):
        return cfg
    # A scalar (string, list) is an event with default behaviour and
    # no `types:` filtering; treat as the bare form.
    return {}


def has_checks_requested(cfg):
    """True if the merge_group config includes `checks_requested` in
    its `types:` list. A bare `merge_group:` (no `types:` key) accepts
    every type and returns True: the workflow will run for
    `checks_requested` along with the other merge_group event types.
    """
    if not isinstance(cfg, dict):
        return False
    types = cfg.get("types")
    if types is None:
        return True
    if isinstance(types, list):
        return "checks_requested" in types
    if isinstance(types, str):
        return types == "checks_requested"
    return False


def is_required_emitter(spec, name):
    """True if the workflow's jobs emit any develop-required check
    name, either by `uses:`-ing a reusable in
    DEVELOP_REQUIRED_REUSABLES or by carrying a job `name:` in
    DIRECT_EMITTER_NAMES. The reusable-emit map is built once by
    `main()` and passed through; this helper only consults it.
    """
    jobs = spec.get("jobs") or {}
    if not isinstance(jobs, dict):
        return False
    for _job_id, job in jobs.items():
        if not isinstance(job, dict):
            continue
        job_name = job.get("name")
        if isinstance(job_name, str) and job_name in DIRECT_EMITTER_NAMES:
            return True
        uses = job.get("uses")
        if isinstance(uses, str) and uses.startswith("./"):
            ref = uses[2:]
            for entry in DEVELOP_REQUIRED_REUSABLES:
                if ref.endswith(entry):
                    return True
    return False


def main():
    files = sorted(set(
        glob.glob(".github/workflows/*.yml")
        + glob.glob(".github/workflows/*.yaml")
    ))

    # First pass: read every workflow. A non-reusable workflow that
    # emits a required check is "an emitter"; it must have
    # `merge_group` in its `on:`. A workflow (reusable or not) that
    # carries `merge_group` for any other reason is refused.
    # Reusables themselves are checked for being emitters so a
    # workflow that has the wrong caller shape (someone uses a
    # reusable from a non-reusable file) still gets caught: the
    # caller scan below would not see the reusable's emission, but
    # the requirement is that SOMETHING in the tree triggers the
    # check, and the reusable's own `workflow_call` does not.
    emitter_paths = []
    merge_group_paths = []
    parsed = {}
    for path in files:
        name = os.path.basename(path)
        try:
            with open(path) as fh:
                spec = yaml.safe_load(fh)
        except yaml.YAMLError as e:
            # Warn and skip, the same way caller-if and
            # caller-with-defaults do. Silent skip would let a
            # misconfigured workflow's emission (or absence) hide;
            # the warning keeps the silence visible to whoever is
            # reading the run.
            print(f"::warning file={path}::skipped: YAML parse error: {e}")
            continue
        if not isinstance(spec, dict):
            continue
        parsed[path] = spec
        on = triggers(spec)
        if merge_group_cfg(on) is not None:
            merge_group_paths.append((path, name))
        # An emitter is a non-reusable workflow that emits a
        # develop-required check. A reusable is a reusable, and its
        # caller (not the reusable itself) is the workflow that
        # must carry the trigger.
        if name.startswith("reusable-"):
            continue
        if is_required_emitter(spec, name):
            emitter_paths.append((path, name))

    violations = []

    # Pass 1: emitters must carry `merge_group` with `checks_requested`
    # in `types:`. A bare `merge_group:` (no `types:`) is accepted.
    for path, name in emitter_paths:
        spec = parsed[path]
        on = triggers(spec)
        cfg = merge_group_cfg(on)
        if cfg is None:
            violations.append((
                path,
                f"`{name}` emits a develop-required check (via `uses: ./` "
                f"of one of {sorted(DEVELOP_REQUIRED_REUSABLES)} or a job "
                f"`name:` in {sorted(DIRECT_EMITTER_NAMES)}) but has no "
                f"`merge_group` trigger. The develop ruleset requires the "
                f"check in the merge queue; without `merge_group` the "
                f"check is not reported and the queue never merges.",
            ))
            continue
        if not has_checks_requested(cfg):
            types = cfg.get("types")
            violations.append((
                path,
                f"`{name}` emits a develop-required check and carries "
                f"`merge_group`, but its `types:` is {types!r} and does "
                f"not include `checks_requested`. The queue fires "
                f"`checks_requested`; a workflow that only listens for "
                f"other merge_group types will not run in the queue.",
            ))

    # Pass 2: any workflow carrying `merge_group` that is NOT an
    # emitter is refused. The reason is operational: release.yml
    # holds publishing secrets, and a workflow that adds
    # `merge_group` for no reason would boot inside the queue and
    # cost a runner. The rule also catches the case where someone
    # adds the trigger "just in case" and forgets to remove it.
    emitter_path_set = {p for p, _ in emitter_paths}
    for path, name in merge_group_paths:
        if path in emitter_path_set:
            continue
        # Reusables themselves are never emitters (their on: is
        # workflow_call); the rule still fires on them so a
        # reusable that adds the trigger to its own file is
        # caught, since the trigger has no caller that would
        # benefit.
        violations.append((
            path,
            f"`{name}` carries `merge_group` but does not emit a "
            f"develop-required check. A workflow that runs in the "
            f"queue without being asked to spends a runner and, for "
            f"release.yml, exposes publishing secrets to the queue. "
            f"Remove the `merge_group` trigger unless this workflow "
            f"emits one of {sorted(DEVELOP_REQUIRED_REUSABLES | DIRECT_EMITTER_NAMES)}.",
        ))

    if violations:
        for path, msg in violations:
            print(f"::error file={path}::{msg}")
        sys.exit(1)

    # Fail closed on an empty scan: a tree with no emitters at all
    # means the ruleset cannot be enforced, and "no violations" would
    # be a pass that did not look. The check is over the entire
    # workflow tree, not a per-file assertion, so a "found nothing"
    # answer is itself a defect.
    if not emitter_paths:
        print(
            "::error::merge-group-triggers: no workflow in .github/workflows/ "
            "emits a develop-required check. The scan cannot prove the "
            "triggers are correct; verify the ruleset and the "
            "DEVELOP_REQUIRED_REUSABLES / DIRECT_EMITTER_NAMES maps."
        )
        sys.exit(1)

    print(
        "merge-group-triggers: every develop-required check has a "
        "caller that triggers on `merge_group` (with `checks_requested` "
        "where `types:` is set), and no other workflow carries the trigger."
    )


if __name__ == "__main__":
    main()
