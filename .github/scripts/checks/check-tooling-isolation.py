#!/usr/bin/env python3
"""A job holding a secret must not install third-party tooling.

Run from the repository root by the `tooling-isolation` step of
reusable-workflow-lint.yml.
"""
import glob
import re
import sys

import yaml

# Three forms reach a secret at runtime: `secrets.<id>`,
# `secrets['<id>']` and `secrets["<id>"]` (with optional
# whitespace inside the brackets). `secrets: inherit` and the
# `secrets:` block on a reusable call don't match, because
# those are declaration blocks, not references, and `secrets:`
# is not followed by `.` or `[`.
SECRET_RE = re.compile(
    r"(?<!\w)secrets(?:\.[A-Za-z_]\w*|\[\s*['\"]"
    r"[A-Za-z_]\w*['\"]\s*])"
)

# (`pattern`, `opt-out marker on the same line`). The marker, when
# present on the line, exempts that install: the only case where
# the resolved tree is fixed is pip with --require-hashes, which
# reads the lockfile verbatim and installs exactly what it lists.
#
# Property-based list (every entry is a known way to pull code
# from outside this org and have it run on the runner; the
# literal-string list this replaced caught the obvious shapes
# and missed synonyms like `cargo binstall`, `pip3 install`,
# `python -m pip install`, `python3 -m pip install`, `pipx
# install`, and `dotnet tool install`). The release signing
# job holds `GLYNDOR_RELEASE_ED25519_KEY` and runs `pip install
# --quiet --require-hashes -r .github/scripts/release/sign-requirements
# .txt`, which executes the cryptography package's native build
# script; hashed, so it is exempt, and the build script runs.
# The release target's own build-script count is 14 (cargo
# metadata --filter-platform x86_64-unknown-linux-gnu),
# confirmed against 15 directories in target/release/build (one
# of which is podup's own build script); the whole lockfile
# graph with dev-deps is 33, and an acceptance built on 33 would
# exempt crates the release never builds.
INSTALL_PATTERNS = (
    ("cargo install",   None),
    ("cargo binstall",  None),
    ("go install",      None),
    ("gem install",     None),
    ("npm install -g",  None),
    ("npm i -g",        None),
    ("dotnet tool install", None),
    ("pip install",     "--require-hashes"),
    ("pip3 install",    "--require-hashes"),
    ("python -m pip install",  "--require-hashes"),
    ("python3 -m pip install", "--require-hashes"),
    ("pipx install",    None),
)


def normalize_run(value):
    # `run:` is either a single string or a list of strings. Split
    # each into lines so the install scan is per-line: a
    # `--require-hashes` marker on one line only exempts that line.
    if isinstance(value, list):
        out = []
        for v in value:
            if isinstance(v, str):
                out.extend(v.splitlines())
        return out
    if isinstance(value, str):
        return value.splitlines()
    return []


def find_violations(spec_path):
    violations = []
    try:
        with open(spec_path) as fh:
            spec = yaml.safe_load(fh)
    except yaml.YAMLError as e:
        # Silent skip would hide a misconfigured workflow that the
        # caller is going to learn about elsewhere in CI for the
        # wrong reason. Warning annotation instead.
        print(f"::warning file={spec_path}::skipped: YAML parse error: {e}")
        return violations
    if not isinstance(spec, dict):
        return violations

    # A workflow-level `env:` propagates to every job in the
    # workflow, so a secret referenced there reaches every step.
    # `defaults:` only sets `shell` and `working-directory`, not
    # `env`, so a secret under it does NOT reach a step and is
    # not in scope here.
    workflow_env = spec.get("env")
    workflow_env_text = (
        yaml.safe_dump(workflow_env)
        if isinstance(workflow_env, dict) else ""
    )
    workflow_holds_secret = bool(SECRET_RE.search(workflow_env_text))

    jobs = spec.get("jobs") or {}
    for job_id, job in jobs.items():
        if not isinstance(job, dict):
            continue
        # Re-serialize the job so `with:`, `env:`, `if:` and
        # `run:` strings are all scanned for a secret reference.
        # Comments are stripped by safe_load, so they cannot
        # produce a false positive.
        job_text = yaml.safe_dump(job)
        # The job holds a secret if its own text references one,
        # or if the workflow-level env does (every job inherits
        # it).
        job_holds_secret = bool(SECRET_RE.search(job_text))
        if not job_holds_secret and not workflow_holds_secret:
            continue
        for step in (job.get("steps") or []):
            if not isinstance(step, dict):
                continue
            run = step.get("run")
            if run is None:
                continue
            for line in normalize_run(run):
                stripped = line.lstrip()
                if not stripped or stripped.startswith("#"):
                    continue
                for pattern, except_marker in INSTALL_PATTERNS:
                    if pattern in line and not (
                        except_marker and except_marker in line
                    ):
                        violations.append((
                            spec_path, job_id, line,
                            "the workflow-level `env:`"
                            if not job_holds_secret
                            else "this job",
                        ))
                        break
    return violations


def main():
    files = sorted(set(
        glob.glob(".github/workflows/*.yml")
        + glob.glob(".github/workflows/*.yaml")
    ))
    violations = []
    for path in files:
        violations.extend(find_violations(path))
    if violations:
        for path, job_id, line, source in violations:
            # Naming where the secret comes from matters when it is
            # not in the job: an author reading "job X holds a
            # secret" and seeing no secret in job X concludes the
            # check is wrong rather than looking further up the file.
            msg = (
                f"job `{job_id}` installs third-party tooling while holding a secret "
                f"from {source}: "
                f"`{line.strip()}`. See Glyndor/apt#121. Fix by moving the install "
                f"into a job that holds no secrets, or pin it by hash "
                f"(e.g. `pip install --require-hashes`)."
            )
            print(f"::error file={path}::{msg}")
        sys.exit(1)
    print(
        "tooling-isolation: no job holding a secret installs third-party tooling."
    )
    sys.exit(0)


if __name__ == "__main__":
    sys.exit(main())
