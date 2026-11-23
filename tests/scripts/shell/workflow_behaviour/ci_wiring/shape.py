#!/usr/bin/env python3
"""The exact shape of the workflows that decide what runs and what is certified.

A test that checks a job is *present* passes when its `run:` has been swapped
with another suite's, when a suite is handed a fixed `false`, when the plan is
read from the checkout instead of the base commit, or when a job is added that
the test knows by name. This one holds the whole shape: for every job of the
listed workflows, its name, `needs`, `if`, `uses`, `with`, `permissions`,
`strategy`, and a fingerprint of every step (its `uses`, `with`, `if`, `env`,
`shell` and its script, but not its display name). The set of jobs is closed: a
job that is not in the snapshot is a failure, and so is one that went missing.

Changing what a workflow does means changing the snapshot in the same diff,
which is how it is read as a change to the contract and not as an edit.

    shape.py --check     compare the tree with the snapshot (exit 1 on any difference)
    shape.py --update    write the snapshot

The snapshot is JSON next to this file. In `podman-lane.yml` the VM job is held
by its conditions only (the script that boots the VM is held by the lane's own
tests); everywhere else every step is held whole.
"""
import hashlib
import json
import os
import sys

import yaml

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.environ.get("WIRING_ROOT") or os.path.abspath(os.path.join(HERE, "..", "..", "..", "..", ".."))
SNAPSHOT = os.path.join(HERE, "snapshot.json")
sys.path.insert(0, os.path.join(HERE, "..", "..", "..", "..", "..", ".github", "scripts", "lib"))
import ci_yaml  # noqa: E402  (the one loader: it refuses a duplicate key)

# workflow file -> how its steps are held. "whole": every step; "conditions": the
# `name` and `if` of each step, for a job whose script is held by other tests.
FILES = {
    "ci.yml": {},
    "dco.yml": {},
    "line-limit.yml": {},
    "workflow-lint.yml": {},
    "commit-hygiene.yml": {},
    "main-guard.yml": {},
    "reusable-rust-ci.yml": {},
    "reusable-dco.yml": {},
    "reusable-line-limit.yml": {},
    "reusable-main-guard.yml": {},
    "reusable-cancel-others.yml": {},
    "podman-lane.yml": {"podman-vm": "conditions"},
}


def digest(obj):
    blob = json.dumps(obj, sort_keys=True, separators=(",", ":"), default=str)
    return hashlib.sha256(blob.encode()).hexdigest()[:16]


def step_shape(step, mode):
    if mode == "conditions":
        return {"name": step.get("name"), "if": step.get("if")}
    body = {k: v for k, v in step.items() if k != "name"}
    if "run" in body:
        body["run"] = str(body["run"]).strip()
    return {"name": step.get("name"), "sha": digest(body)}


def job_shape(job, mode):
    shape = {k: job.get(k) for k in ("name", "needs", "if", "uses", "with", "permissions", "strategy", "runs-on", "env")}
    shape["steps"] = [step_shape(s, mode) for s in job.get("steps") or []]
    return {k: v for k, v in shape.items() if v not in (None, [])}


def build():
    out = {}
    for name, modes in FILES.items():
        spec = ci_yaml.load(os.path.join(ROOT, ".github", "workflows", name))
        on = spec.get("on", spec.get(True))
        entry = {
            "on": on,
            "permissions": spec.get("permissions"),
            "env": spec.get("env"),
            "jobs": {},
        }
        for job_id, job in (spec.get("jobs") or {}).items():
            entry["jobs"][job_id] = job_shape(job, modes.get(job_id, "whole"))
        out[name] = json.loads(json.dumps(entry, default=str))
    return out


def diff(want, have, path=""):
    if isinstance(want, dict) and isinstance(have, dict):
        for key in sorted(set(want) | set(have)):
            if key not in have:
                yield f"{path}/{key}: missing from the tree (the snapshot holds it)"
            elif key not in want:
                yield f"{path}/{key}: not in the snapshot (a job or key nobody listed)"
            else:
                yield from diff(want[key], have[key], f"{path}/{key}")
    elif want != have:
        yield f"{path}: the tree has {json.dumps(have, default=str)[:120]}, the snapshot holds {json.dumps(want, default=str)[:120]}"


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else "--check"
    try:
        have = build()
    except yaml.YAMLError as e:
        print(f"a workflow cannot be read: {e}")
        return 1
    if mode == "--update":
        with open(SNAPSHOT, "w") as fh:
            json.dump(have, fh, indent=1, sort_keys=True)
            fh.write("\n")
        print("snapshot written")
        return 0
    want = json.load(open(SNAPSHOT))
    problems = list(diff(want, have))
    for p in problems:
        print(p)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
