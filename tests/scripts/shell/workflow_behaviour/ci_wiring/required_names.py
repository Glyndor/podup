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
    "audit / cargo audit",
    "audit / cargo deny",
    "commit identity",
    "secret scan",
    "pin values",
]
MAIN = [
    "rust / Gate (promotion)",
    "dco / Signed-off-by present on every commit (promotion)",
    "line-limit / line limit (promotion)",
    "workflow-lint / workflow-lint (promotion)",
    "Supported Podman majors (promotion)",
    "audit / cargo audit (promotion)",
    "audit / cargo deny (promotion)",
    "commit identity (promotion)",
    "secret scan (promotion)",
    "pin values (promotion)",
    "develop-only / develop-only",
]
# (event, base branch) for each kind of run a workflow can have. `Develop` and `MAIN`
# are bases that differ from the protected branches only by case: a trigger's
# `branches:` filter tells them apart, an `==` in an expression does not.
CONTEXTS = [
    ("pull_request", "develop"), ("pull_request", "main"), ("pull_request", "Develop"),
    ("pull_request", "MAIN"), ("pull_request", "feature"), ("merge_group", ""),
    ("push", "develop"), ("push", "main"), ("schedule", ""), ("workflow_dispatch", ""),
]
SUFFIX = ("github.event_name == 'pull_request' && github.base_ref == 'main' && ' (promotion)' || "
          "!(github.event_name == 'merge_group' || github.event_name == 'pull_request' && github.base_ref == 'develop') "
          "&& format(' ({0})', github.event_name) || ''")
# Every expression a job name may contain. Anything else is refused: a name this
# does not know how to resolve is a name it cannot hold to the list. The matrix and
# input values are listed because no required name has one.
ALLOWED = {SUFFIX, "matrix.target", "matrix.asset", "matrix.arch", "matrix.os", "inputs.msrv"}
# The jobs whose names keep a matrix or input value after rendering. None is required;
# a job that is not listed and keeps an expression is refused, so a required name cannot
# hide in one.
UNRESOLVED = {("linux-hardening.yml", "hardened"), ("release.yml", "build"), ("release.yml", "sign"),
              ("release.yml", "build-deb"), ("reusable-rust-ci.yml", "test-extra"), ("reusable-rust-ci.yml", "msrv")}
# The workflows whose pull_request trigger may list `edited` and `reopened`, with why: they judge the base
# of the pull request, and an edit can change it. A required check that is one job and is never skipped.
EDIT_EXCEPTIONS = {
    "main-guard.yml": "it judges the base of the pull request (only a pull request into main must come from develop), "
                      "so changing the base to main has to run it; one job of a few seconds that is never cancelled",
}
TOKEN = re.compile(r"\s*(&&|\|\||==|!=|!|\(|\)|,|'[^']*'|github\.event_name|github\.base_ref|format)")


class Cs(str):
    """A string whose `==` and `!=` ignore case, as they do in an expression."""

    def __eq__(self, other):
        return str.lower(self) == str.lower(str(other))

    def __ne__(self, other):
        return not self.__eq__(other)

    __hash__ = str.__hash__


def evaluate(expr, event, base):
    expr = expr.strip()
    py, pos = "", 0
    while pos < len(expr):
        m = TOKEN.match(expr, pos)
        if not m:
            raise ValueError(f"cannot evaluate {expr!r} at {expr[pos:pos + 20]!r}")
        tok, pos = m.group(1), m.end()
        if tok.startswith("'"):
            py += f"_s({tok})"
        else:
            py += {"&&": " and ", "||": " or ", "!": " not ", "github.event_name": f"_s({event!r})",
                   "github.base_ref": f"_s({base!r})", "format": "_format"}.get(tok, tok)
    return eval(py, {"_format": lambda f, *a: Cs(str(f).format(*a)), "_s": Cs})  # noqa: S307  (tokens are whitelisted above)


def render(name, event, base):
    """The name with its expressions run. An expression that is not in ALLOWED is
    an error; a listed one this does not evaluate (a matrix value) is left as written."""
    def one(m):
        if m.group(1).strip() not in ALLOWED:
            raise ValueError(f"the name {name!r} holds an expression that is not in the list of resolved patterns")
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
    """Whether the trigger starts the workflow for this kind of run. The branch
    filters compare exactly, with case."""
    on = on_of(spec)
    if event not in on:
        return False
    cfg = on[event] or {}
    branches = cfg.get("branches") if isinstance(cfg, dict) else None
    if event in ("pull_request", "push") and branches is not None:
        return base in branches
    return True


def expand(specs, spec, prefix, event, base, out, fname, here=None):
    """Every name a workflow's jobs can emit, into the local reusables however deep.
    A job's own `if:` is not read: a skipped job still reports its name."""
    for job_id, job in (spec.get("jobs") or {}).items():
        uses = job.get("uses")
        if isinstance(uses, str) and uses.startswith("./"):
            inner = specs.get(os.path.basename(uses), {})
            expand(specs, inner, f"{prefix}{job_id} / ", event, base, out, fname, os.path.basename(uses))
        else:
            name = render(job.get("name", job_id), event, base)
            if "${{" in name and (here or fname, job_id) not in UNRESOLVED:
                raise ValueError(f"{fname}: job `{job_id}` keeps an expression in its name ({name!r}) that is not listed as not required")
            # Required names are compared without regard to case: two names that differ
            # only by case would be told apart here and could be confused elsewhere.
            out.setdefault((prefix + name).lower(), set()).add((fname, event, base))


def produced(root):
    """name -> set of (workflow, event, base). Raises ValueError on a name it cannot resolve."""
    specs = {}
    for path in ci_yaml.workflows_in(os.path.join(root, ".github", "workflows")):
        specs[os.path.basename(path)] = ci_yaml.load(path)
    out = {}
    for fname, spec in specs.items():
        if fname.startswith("reusable-"):
            continue
        for event, base in CONTEXTS:
            if runs(spec, event, base):
                expand(specs, spec, "", event, base, out, fname)
    return out, specs


def main():
    try:
        made, specs = produced(ROOT)
    except ValueError as e:
        print(e)
        return 1
    problems = []
    # A pull request trigger that feeds a required name runs for a push and not for an edit or a reopening,
    # except the one workflow that judges the base of the pull request.
    for fname, spec in specs.items():
        cfg = on_of(spec).get("pull_request")
        if fname.startswith("reusable-") or cfg is None:
            continue
        feeds = any(f == fname and e == "pull_request" for who in made.values() for f, e, _b in who
                    if any(n.lower() in made and (f, e, _b) in made[n.lower()] for n in DEVELOP + MAIN))
        types = (cfg or {}).get("types") if isinstance(cfg, dict) else None
        branches = (cfg or {}).get("branches") if isinstance(cfg, dict) else None
        if feeds and not (branches and set(branches) <= {"develop", "main"}):
            problems.append(f"{fname}: its pull_request trigger feeds a required name and does not limit the bases "
                            "to `branches: [develop, main]`, which is the only filter that tells `Develop` from `develop`")
        extra = sorted(set(types or []) & {"edited", "reopened"})
        if feeds and types is None:
            problems.append(f"{fname}: its pull_request trigger feeds a required name and lists no `types:`; the default includes `reopened`, "
                            "which starts a second set of runs of the same commit and cancels the first")
        elif feeds and fname not in EDIT_EXCEPTIONS and extra:
            problems.append(f"{fname}: its pull_request trigger feeds a required name and lists {extra}: a second set of runs of the same "
                            "commit cancels the first, and the aggregators of the cancelled one report a failure under the required names")
        elif feeds and fname in EDIT_EXCEPTIONS:
            jobs = spec.get("jobs") or {}
            always = [j for j, v in jobs.items() if "always()" in str((v or {}).get("if", ""))]
            if sorted(extra) != ["edited", "reopened"]:
                problems.append(f"{fname}: it is on the list of workflows that judge the base but does not list `edited` and `reopened`; the list is stale")
            if len(jobs) != 1 or always:
                problems.append(f"{fname}: it lists `edited` and has {len(jobs)} job(s){' and an `always()` aggregator' if always else ''}; "
                                "only a single job that is never skipped may")
            if (spec.get("concurrency") or {}).get("cancel-in-progress") is not False:
                problems.append(f"{fname}: it lists `edited` and cancels what is in progress; a cancelled run leaves a red check under the required name")
    for fname, reason in EDIT_EXCEPTIONS.items():
        if fname not in specs or not reason.strip():
            problems.append(f"{fname}: it is on the list of workflows that may list `edited` and is missing or has no reason")
    # The security results are never advisory and never cancelled: they run in the merge
    # queue, and their concurrency group does not cancel what is in progress.
    for fname in ("audit.yml", "reusable-rust-audit.yml", "commit-hygiene.yml"):
        cancel = (specs[fname].get("concurrency") or {}).get("cancel-in-progress")
        if cancel is not False:
            problems.append(f"{fname}: `cancel-in-progress` is {cancel!r}; a security result is never cancelled by a newer run")
    for name in DEVELOP:
        if not any(e == "merge_group" for _w, e, _b in made.get(name.lower(), set())):
            problems.append(f"{name}: not produced in the merge queue, so it cannot be required there")
    for name in DEVELOP:
        who = made.get(name.lower(), set())
        if not who:
            problems.append(f"{name}: no workflow produces it")
            continue
        if len({w for w, _e, _b in who}) > 1:
            problems.append(f"{name}: produced by more than one workflow: {sorted({w for w, _e, _b in who})}")
        bad = sorted((e, b) for _w, e, b in who if e not in ("pull_request", "merge_group") or b not in ("develop", ""))
        if bad:
            problems.append(f"{name}: also produced by {bad}; a push, a schedule, a dispatch or a pull request into main must not answer for it")
    for name in MAIN:
        who = made.get(name.lower(), set())
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
