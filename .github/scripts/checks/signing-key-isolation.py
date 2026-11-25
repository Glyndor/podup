#!/usr/bin/env python3
"""Regression guardrail for the jobs of release.yml that hold the signing key.

Usage: signing-key-isolation.py [WORKFLOW_DIR]
       signing-key-isolation.py --print-hashes [WORKFLOW_DIR]

Not a proof of security: a fixed definition and a tripwire on it. The jobs
`sign`, `sign-packages`, `seal`, `attest` and `release` of release.yml are pinned whole. The SHA-256 of
each one's canonical JSON (the job as loaded, plus the workflow's `env`,
`defaults` and `permissions`, which apply to it) must equal the hash in
`release-pinned.txt`. Any change to any field of those jobs, a comment in a
`run:` included, fails until the hash is regenerated with `--print-hashes` in
the same pull request, which is where the whole diff of the job gets read.

Around that, in every workflow: the key (`secrets.GLYNDOR_RELEASE_ED25519_KEY`,
`toJSON(secrets)`, a computed `secrets[...]`, `secrets: inherit`) appears only
in `sign`, `sign-packages` and `seal` of release.yml, read inside a `${{ }}`
expression, and each of them declares `environment: release`, which the environment restricts
to `main` and the `v*` tags (where the secret is kept is the repository's setting, not this
file's); the job that writes the release (`release`) never holds it;
`id-token: write` and `attestations: write` only in `attest`. Exit 0 or 1.
"""
import hashlib
import json
import os
import re
import sys

import yaml

sys.path.insert(0, ".github/scripts/lib")
import ci_yaml  # noqa: E402  (the one loader and enumerator; run from the repository root)

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
PINNED = ("sign", "sign-packages", "seal", "attest", "release")
# The jobs that sign and seal hold the key and have no right to write a release; the job that
# writes the release has the right and never sees the key. The two never meet in one job.
KEY_JOBS = ("sign", "sign-packages", "seal")
ENVIRONMENT = "release"


def strings(value):
    """Every string in the parsed object, keys and values, as loaded.

    The check used to read a YAML dump of the job, and the dump writes a tab
    or a newline before `secrets` as the two characters backslash and t, which
    glues them to the word and hides it from a word boundary. The strings are
    read as the parser gives them instead.
    """
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for k, v in value.items():
            yield from strings(k)
            yield from strings(v)
    elif isinstance(value, (list, tuple)):
        for item in value:
            yield from strings(item)


def holds_key(value):
    return any(KEY.search(e) for text in strings(value) for e in EXPR.findall(text))


def digest(spec, job):
    shared = {k: spec.get(k) for k in ("env", "defaults", "permissions")}
    # The commit an action is pinned to is what Dependabot moves, so it is not part of the
    # definition held here; the action, that it is pinned and every other property are.
    blob = json.dumps({"job": ci_yaml.normalize_pins(spec["jobs"][job]), "workflow": shared}, sort_keys=True, separators=(",", ":"), default=str)
    return hashlib.sha256(blob.encode()).hexdigest()


def pinned():
    lines = open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "release-pinned.txt"))
    return dict(l.split() for l in lines if l.strip() and not l.startswith("#"))


def offenders(path):
    spec = ci_yaml.load(path)
    if not isinstance(spec, dict):
        return []
    is_release = os.path.basename(path) == "release.yml"
    found = []
    if holds_key(spec.get("env") or {}):
        found.append("workflow: holds the key in its environment")
    for name, job in (spec.get("jobs") or {}).items():
        if (job.get("secrets") == "inherit" or holds_key(job)) and not (is_release and name in KEY_JOBS):
            found.append(f"job `{name}`: holds the key, which only {', '.join(f'`{k}`' for k in KEY_JOBS)} of release.yml may")
        if is_release and name in KEY_JOBS and holds_key(job) and job.get("environment") != ENVIRONMENT:
            found.append(f"job `{name}`: reads the key without `environment: {ENVIRONMENT}`")
        if is_release and name in KEY_JOBS and "write" in (job.get("permissions") or {}).values():
            found.append(f"job `{name}`: holds the key and a permission to write, which are never in one job")
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
        spec = ci_yaml.load(f"{root}/release.yml")
        print("\n".join(f"{j} {digest(spec, j)}" for j in PINNED))
        return 0
    files = ci_yaml.workflows_in(root)
    if not files:
        print(f"signing-key-isolation: no workflow under {root}", file=sys.stderr)
        return 2
    bad = [o for f in files for o in offenders(f)]
    print("\n".join(bad) if bad else "signing-key-isolation: the pinned release jobs are unchanged.")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
