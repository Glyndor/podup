#!/usr/bin/env python3
"""No required check name is produced by more than one workflow or more than one kind of run.

GitHub matches a required check by its name on a commit. The commit a merge queue
lands on `develop` is the commit that the push to `develop` runs on and the head of
the pull request that promotes `develop` into `main`, so a check of the same name
from the push (whose plan looked at the diff of that one push) could answer for the
pull request into `main`. This lists, for every workflow, every name a job can emit
in every kind of run (a pull request into `develop`, a pull request into `main`,
the merge queue, a push to either branch, a schedule, a dispatch), and holds the
required names to what the rulesets ask:

  develop   produced by one workflow, only by a pull request into `develop` and by
            the merge queue (two runs on different commits)
  main      produced by one workflow, only by a pull request whose base is `main`
  neither   a push, a schedule or a dispatch never produces a required name

Names are evaluated, not read: a job name may end in an expression, and the
expression is run for each kind of run with a small evaluator that knows the
names, `&&`, `||`, `==`, `!=` and `format` and refuses anything else.
"""
import os
import re
import sys

import yaml

ROOT = os.environ.get("WIRING_ROOT") or os.path.abspath(
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "..", "..", "..")
)
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "..", "..", "..", ".github", "scripts", "lib"))
import ci_yaml  # noqa: E402

DEVELOP = [
    "rust / Gate",
    "dco / Signed-off-by present on every commit",
    "line-limit / line limit",
    "workflow-lint / workflow-lint",
    "Supported Podman majors",
]
MAIN = [
    "rust / Gate (promotion)",
    "dco / Signed-off-by present on every commit (promotion)",
    "line-limit / line limit (promotion)",
    "workflow-lint / workflow-lint (promotion)",
    "Supported Podman majors (promotion)",
    "develop-only / develop-only",
]
# (event, base branch) for each kind of run a workflow can have.
CONTEXTS = [
    ("pull_request", "develop"), ("pull_request", "main"), ("merge_group", ""),
    ("push", "develop"), ("push", "main"), ("schedule", ""), ("workflow_dispatch", ""),
]
TOKEN = re.compile(r"\s*(&&|\|\||==|!=|\(|\)|,|'[^']*'|github\.event_name|github\.base_ref|format)")


def evaluate(expr, event, base):
    expr = expr.strip()
    py, pos = "", 0
    while pos < len(expr):
        m = TOKEN.match(expr, pos)
        if not m:
            raise ValueError(f"cannot evaluate {expr!r} at {expr[pos:pos + 20]!r}")
        tok, pos = m.group(1), m.end()
        py += {"&&": " and ", "||": " or ", "github.event_name": repr(event),
               "github.base_ref": repr(base), "format": "_format"}.get(tok, tok)
    return eval(py, {"_format": lambda f, *a: f.format(*a)})  # noqa: S307  (tokens are whitelisted above)


def render(name, event, base):
    """The name with its expressions run. An expression this does not know (a
    matrix value) is left as written: no required name has one, and a required name
    that did would no longer match what the list holds."""
    def one(m):
        try:
            return str(evaluate(m.group(1), event, base))
        except ValueError:
            return m.group(0)
    return re.sub(r"\$\{\{(.*?)\}\}", one, name)


def on_of(spec):
    on = spec.get("on", spec.get(True))
    if isinstance(on, str):
        return {on: {}}
    if isinstance(on, list):
        return {e: {} for e in on}
    return on or {}


def runs(spec, event, base):
    on = on_of(spec)
    if event not in on:
        return False
    cfg = on[event] or {}
    branches = cfg.get("branches") if isinstance(cfg, dict) else None
    if event in ("pull_request", "push") and branches is not None:
        return base in branches
    return True


def produced(root):
    """name -> set of (workflow, event, base)."""
    specs = {}
    for path in ci_yaml.workflows_in(os.path.join(root, ".github", "workflows")):
        specs[os.path.basename(path)] = ci_yaml.load(path)
    out = {}
    for fname, spec in specs.items():
        if fname.startswith("reusable-"):
            continue
        for event, base in CONTEXTS:
            if not runs(spec, event, base):
                continue
            for job_id, job in (spec.get("jobs") or {}).items():
                uses = job.get("uses")
                if isinstance(uses, str) and uses.startswith("./"):
                    inner = specs.get(os.path.basename(uses), {})
                    for ijob_id, ijob in (inner.get("jobs") or {}).items():
                        if "name" in ijob:
                            name = f"{job_id} / {render(ijob['name'], event, base)}"
                            out.setdefault(name, set()).add((fname, event, base))
                else:
                    name = render(job.get("name", job_id), event, base)
                    out.setdefault(name, set()).add((fname, event, base))
    return out


def main():
    made = produced(ROOT)
    problems = []
    for name in DEVELOP:
        who = made.get(name, set())
        if not who:
            problems.append(f"{name}: no workflow produces it")
            continue
        if len({w for w, _e, _b in who}) > 1:
            problems.append(f"{name}: produced by more than one workflow: {sorted({w for w, _e, _b in who})}")
        bad = sorted((e, b) for _w, e, b in who if e not in ("pull_request", "merge_group") or b == "main")
        if bad:
            problems.append(f"{name}: also produced by {bad}; a push, a schedule, a dispatch or a pull request into main must not answer for it")
    for name in MAIN:
        who = made.get(name, set())
        if not who:
            problems.append(f"{name}: no workflow produces it")
            continue
        if len({w for w, _e, _b in who}) > 1:
            problems.append(f"{name}: produced by more than one workflow: {sorted({w for w, _e, _b in who})}")
        bad = sorted((e, b) for _w, e, b in who if (e, b) != ("pull_request", "main"))
        if bad:
            problems.append(f"{name}: also produced by {bad}; only a pull request into main may produce it")
    for p in problems:
        print(p)
    if not problems:
        print(f"required-names: {len(DEVELOP)} names for develop and {len(MAIN)} for main, each from one workflow and the runs that should")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
