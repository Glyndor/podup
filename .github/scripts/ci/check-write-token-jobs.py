#!/usr/bin/env python3
"""Hold every job that can end other runs (`actions: write`) to an exact list.

The token that can cancel runs is the most dangerous one a pull request's
workflow can hold: whatever runs in that job can end the runs of any workflow.
The rule is that such a job runs only steps this file lists, and that nothing
in them runs a file from the pull request's tree.

What counts as holding the permission, by effective permissions and not by the
spelling:

  - `permissions: write-all` on the job or on the workflow,
  - `actions: write` on the job or on the workflow,
  - a job with no `permissions` of its own that inherits either from its
    workflow, and a job of a called reusable workflow that has none and whose
    reusable declares none either, which inherits what the calling job holds.

A workflow-level grant is refused outright: the permission belongs to one job.
A job that holds it is compared, step by step, with KNOWN below: the same
number of steps, and each one identical to the fingerprint listed (its `uses`
and `with` for an action, its whole text for a script). The checkout has to be
pinned by hash. A job that calls a reusable workflow while holding the
permission may call only the reusables listed in CALLABLE.

Changing a listed step is a change to this file, in the same diff, which is the
point: it is read as a change to the permission and not as an edit of a
workflow.

Run it with `--print` to see the fingerprints of the jobs that hold the
permission now.

Exit 0 when clean, 1 on any violation.
"""
import hashlib
import json
import os
import re
import sys

import yaml

sys.path.insert(0, ".github/scripts/lib")
import ci_yaml  # noqa: E402  (the one loader and enumerator; run from the repository root)

WORKFLOWS = ".github/workflows"

# Reusable workflows a job that holds the permission may call.
CALLABLE = {"reusable-cancel-others.yml"}

# (workflow file, job id) -> (fingerprint of the surroundings, fingerprints of
# its steps in order). A step is
# fingerprinted by everything that decides what it does, except its `name`.
KNOWN = {
    # The cancel job: the checkout pinned by hash, then the launcher that runs
    # the base commit's copy of the script.
    ("reusable-cancel-others.yml", "cancel"): (
        "c58e46bd3d54af68894abe234cf7a63bc17a7ead913b90b632645d4724e55d96",
        [
            "d52a24e6b943d9298499b27f83edef10691c7be1830fc7c01947d6d1497f083e",
            "466c469c207f224b857fe3d7576fb5ce038117f94de4331245c52508dc2af6ce",
        ],
    ),
    # The nightly audit launcher: no checkout, and `gh` only. It runs on
    # `schedule` and never on a pull request.
    ("audit.yml", "audit-develop"): (
        "8d1e0086950fb5bdd28b01835d1157576df62250495783fcf7f2581aa596a254",
        ["27fe53f615eac1599ff272f97abbd2910e2674afd28d0287d5b77a2ecd12e1b6"],
    ),
}

SHA_RE = re.compile(r"^[^@\s]+@[0-9a-f]{40}$")


def load(path):
    spec = ci_yaml.load(path)
    return spec if isinstance(spec, dict) else {}


def holds(perms):
    """True when a `permissions` value grants `actions: write`."""
    if perms == "write-all":
        return True
    if isinstance(perms, dict):
        return perms.get("actions") == "write"
    return False


def fingerprint(step):
    body = {k: v for k, v in step.items() if k != "name"}
    if "run" in body:
        body["run"] = str(body["run"]).strip()
    blob = json.dumps(body, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(blob.encode()).hexdigest()


def context(job, spec):
    """What surrounds the steps and can change them: the environment, defaults,
    container and services of the job, and the environment and defaults of the
    workflow. A step that is identical runs differently under another `env`."""
    body = {
        "job": {k: job.get(k) for k in ("env", "defaults", "container", "services")},
        "workflow": {k: spec.get(k) for k in ("env", "defaults")},
    }
    return hashlib.sha256(json.dumps(body, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def effective(job, spec, inherited):
    if "permissions" in job:
        return job["permissions"]
    if "permissions" in spec:
        return spec["permissions"]
    return inherited


def main():
    files = ci_yaml.workflows_in(WORKFLOWS)
    try:
        specs = {os.path.basename(f): load(f) for f in files}
    except yaml.YAMLError as e:
        print(f"::error::YAML parse error: {e}")
        return 1
    errors = []

    def err(name, job, msg):
        errors.append(f"::error file={WORKFLOWS}/{name}::job `{job}`: {msg}")

    # What a calling job holds, by the file it calls.
    grants = {}
    for name, spec in specs.items():
        if holds(spec.get("permissions")):
            errors.append(
                f"::error file={WORKFLOWS}/{name}::`permissions` at the top of the "
                "workflow grants `actions: write` to every job in it; the permission "
                "belongs to one job"
            )
        for job_id, job in (spec.get("jobs") or {}).items():
            if not isinstance(job, dict):
                continue
            uses = job.get("uses")
            if isinstance(uses, str) and uses.startswith("./"):
                held = holds(effective(job, spec, None))
                grants.setdefault(os.path.basename(uses), []).append((name, job_id, held))

    printed = []
    for name, spec in specs.items():
        inherited = None
        if name in grants:
            # A reusable is called by several jobs; it inherits the strongest.
            if any(held for _f, _j, held in grants[name]):
                inherited = {"actions": "write"}
        for job_id, job in (spec.get("jobs") or {}).items():
            if not isinstance(job, dict):
                continue
            if not holds(effective(job, spec, inherited)):
                continue
            uses = job.get("uses")
            if isinstance(uses, str):
                target = os.path.basename(uses)
                if not uses.startswith("./") or target not in CALLABLE:
                    err(name, job_id, f"holds `actions: write` and calls `{uses}`, which is not in CALLABLE")
                continue
            steps = job.get("steps") or []
            entry = KNOWN.get((name, job_id))
            known = entry[1] if entry else None
            if "--print" in sys.argv:
                known = known if known is not None else []
                entry = entry or ("", [])
            printed.append((name, job_id, [context(job, spec)] + [fingerprint(s) for s in steps]))
            if known is None:
                err(name, job_id, "holds `actions: write` (explicit, `write-all` or inherited) and is not in KNOWN")
                continue
            if context(job, spec) != entry[0]:
                err(name, job_id, "its `env`, `defaults`, `container` or `services`, or the workflow's `env` or `defaults`, are not the ones the list holds")
                continue
            if len(steps) != len(known):
                err(name, job_id, f"has {len(steps)} steps, the list holds {len(known)}")
                continue
            for index, (step, want) in enumerate(zip(steps, known), start=1):
                uses = step.get("uses")
                if isinstance(uses, str) and not SHA_RE.match(uses):
                    err(name, job_id, f"step {index} uses `{uses}`, which is not pinned by hash")
                if fingerprint(step) != want:
                    err(name, job_id, f"step {index} is not the step the list holds")

    if "--print" in sys.argv:
        for name, job_id, prints in printed:
            print(name, job_id)
            for p in prints:
                print("   ", p)
        return 0
    if errors:
        print("\n".join(errors))
        return 1
    print(
        f"write-token-jobs: {len(printed)} job(s) hold `actions: write`, each with "
        "exactly the steps the list holds, and no workflow grants it at the top."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
