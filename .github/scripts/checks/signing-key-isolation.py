#!/usr/bin/env python3
"""Regression guardrail for the jobs of release.yml that hold the signing key.

Usage: signing-key-isolation.py [WORKFLOW_DIR]
       signing-key-isolation.py --print-hashes [WORKFLOW_DIR]

Not a proof of security: a fixed definition and a tripwire on it. The jobs
`sign`, `attest` and `release` of release.yml are pinned whole. The SHA-256 of
each one's canonical JSON (the job as loaded, plus the workflow's `env`,
`defaults` and `permissions`, which apply to it) must equal the hash in
`release-pinned.txt`. Any change to any field of those jobs, a comment in a
`run:` included, fails until the hash is regenerated with `--print-hashes` in
the same pull request, which is where the whole diff of the job gets read.

Around that, in every workflow: the key (`secrets.GLYNDOR_RELEASE_ED25519_KEY`,
`toJSON(secrets)`, a computed `secrets[...]`, `secrets: inherit`) appears only
in `sign` and `release` of release.yml, read inside a `${{ }}` expression;
`id-token: write` and `attestations: write` only in `attest`. Exit 0 or 1.
"""
import glob
import hashlib
import json
import os
import re
import sys

import yaml

# Expression context names, function names and secret names are all case
# insensitive in GitHub Actions: `SECRETS.release_ed25519_key`, `tojson(secrets)`
# and `Secrets['Release_Ed25519_Key']` read the same secret as the canonical
# spelling, so the match has to be case insensitive too.
# Any use of the `secrets` context in an expression counts as reaching for the
# key, except the job token, which is not the signing key. The earlier pattern
# listed spellings of the key (`secrets.NAME`, `secrets['NAME']`,
# `toJSON(secrets)`) and every spelling it did not list got through, such as
# `toJSON((secrets))` or `secrets . NAME`. A job that has no business with the
# key has no business with the context either, so the pattern names the context
# and allows only the token. Names are matched without regard to case because
# GitHub Actions reads them that way.
KEY = re.compile(r"\bsecrets\b(?!\s*\.\s*GITHUB_TOKEN\b)", re.IGNORECASE)
EXPR = re.compile(r"\$\{\{(.*?)\}\}", re.S)
PINNED = ("sign", "attest", "release")
KEY_JOBS = ("sign", "release")


def holds_key(value):
    return bool(KEY.search(" ".join(EXPR.findall(yaml.safe_dump(value, width=10**6)))))


def digest(spec, job):
    shared = {k: spec.get(k) for k in ("env", "defaults", "permissions")}
    blob = json.dumps({"job": spec["jobs"][job], "workflow": shared}, sort_keys=True, separators=(",", ":"), default=str)
    return hashlib.sha256(blob.encode()).hexdigest()


def pinned():
    lines = open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "release-pinned.txt"))
    return dict(l.split() for l in lines if l.strip() and not l.startswith("#"))


def offenders(path):
    spec = yaml.safe_load(open(path))
    if not isinstance(spec, dict):
        return []
    is_release = os.path.basename(path) == "release.yml"
    found = []
    if holds_key(spec.get("env") or {}):
        found.append("workflow: holds the key in its environment")
    for name, job in (spec.get("jobs") or {}).items():
        if (job.get("secrets") == "inherit" or holds_key(job)) and not (is_release and name in KEY_JOBS):
            found.append(f"job `{name}`: holds the key, which only `sign` and `release` of release.yml may")
        grants = job.get("permissions", spec.get("permissions"))
        mints = grants == "write-all" or (isinstance(grants, dict) and "write" in (grants.get("id-token"), grants.get("attestations")))
        if is_release and mints and name != "attest":
            found.append(f"job `{name}`: can mint a token, which only `attest` may")
    if is_release:
        want = pinned()
        found += [f"job `{j}` changed: review the whole diff of the job (it is privileged) and regenerate "
                  "its hash with --print-hashes in the same pull request"
                  for j in PINNED if j not in spec["jobs"] or digest(spec, j) != want.get(j)]
    return [f"::error file={path}::{f}" for f in found]


def main(argv):
    args = [a for a in argv[1:] if a != "--print-hashes"]
    root = args[0] if args else ".github/workflows"
    if "--print-hashes" in argv:
        spec = yaml.safe_load(open(f"{root}/release.yml"))
        print("\n".join(f"{j} {digest(spec, j)}" for j in PINNED))
        return 0
    files = sorted(glob.glob(f"{root}/*.yml") + glob.glob(f"{root}/*.yaml"))
    if not files:
        print(f"signing-key-isolation: no workflow under {root}", file=sys.stderr)
        return 2
    bad = [o for f in files for o in offenders(f)]
    print("\n".join(bad) if bad else "signing-key-isolation: the pinned release jobs are unchanged.")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
