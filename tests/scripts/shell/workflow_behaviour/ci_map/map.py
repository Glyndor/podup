#!/usr/bin/env python3
"""The map of what a change triggers, held as an exact list, in both directions.

Five judges decide whether a lane has anything to look at: the Rust lanes
(`rust-ci-inputs.sh`), the root build the MSRV and doc jobs read
(`rust-build-inputs.sh`), the Debian package (`debian-ci-inputs.sh`), the
workflow lint (`workflow-lint-inputs.sh`) and the Podman VM
(`podman-lane-paths.sh`). Each one is an allow-list that fails closed, and each
has tests for its own rules, but nothing held the whole: a file moved to the
"nothing to run" side of one judge changed no test that read another judge.

This holds the answer of every judge for every tracked file that is not a
document, as a list of patterns in `expected.txt`:

    pattern <TAB> rust build debian lint vm

    rust   R every lane   D the documentation lane   C the CI-only lane   N nothing
    build  B build        N nothing
    debian B build        N nothing
    lint   R run          N nothing
    vm     V boots        N nothing

The workflows with a `paths:` filter are held the same way in `triggers.txt`,
for every tracked file that is not a document: the file, then the workflows
that start for it, by file name without `.yml`, comma separated, `-` for none.
That list is exact too: removing a workflow from the filter of the suite that
watches it, or adding a path that starts a suite for nothing, changes a row.

The check goes both ways. A tracked file that matches no pattern fails: a file
no suite was asked about. A pattern whose file gets another answer from a judge
fails, whichever side moved: a file that can change the code and was moved to
"nothing", or a lane that now runs for a change that is none of its business.
The first pattern that matches a file decides, so the list is read top down.

The files under internal/ and tests/ are many and answered by a handful of
rules, so one file per directory, every shell and PowerShell test, and every
file outside those two trees are judged; the patterns cover all of them.

    map.py --check     compare (exit 1 on any difference)
    map.py --update    rewrite expected.txt from what the judges answer now
                       (review the diff: every line moved to N is a claim)
"""
import fnmatch
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.environ.get("MAP_ROOT") or os.path.abspath(os.path.join(HERE, "..", "..", "..", "..", ".."))
EXPECTED = os.path.join(HERE, "expected.txt")
TRIGGERS = os.path.join(HERE, "triggers.txt")
sys.path.insert(0, os.path.join(HERE, "..", "..", "..", "..", "..", ".github", "scripts", "lib"))
import ci_yaml  # noqa: E402  (the one loader and the one enumerator of workflows)
CHECKS = os.environ.get("MAP_CHECKS") or os.path.join(ROOT, ".github", "scripts", "checks")
WORKFLOWS = os.environ.get("MAP_WORKFLOWS") or os.path.join(ROOT, ".github", "workflows")
DOCS = ("README.md", "CONTRIBUTING.md")


def is_document(path):
    return path in DOCS or (path.startswith("docs/") and path.endswith(".md"))


def tracked():
    out = subprocess.run(["git", "-C", ROOT, "ls-files"], capture_output=True, text=True, check=True).stdout
    return [p for p in out.splitlines() if p and not is_document(p)]


def sample(files):
    """Every file outside internal/ and tests/, every shell and PowerShell test,
    and the first file of each directory below those two trees."""
    picked, seen = [], set()
    for f in files:
        top = f.split("/")[0]
        if top not in ("internal", "tests") or f.startswith("tests/scripts/"):
            picked.append(f)
            continue
        key = "/".join(f.split("/")[:-1])
        if key not in seen:
            seen.add(key)
            picked.append(f)
    return picked


def path_regex(pattern):
    """A GitHub `paths:` pattern as a regular expression: `**` crosses
    directories, `*` and `?` do not."""
    out, i = "", 0
    while i < len(pattern):
        if pattern.startswith("**", i):
            out += ".*"
            i += 2
        elif pattern[i] == "*":
            out += "[^/]*"
            i += 1
        elif pattern[i] == "?":
            out += "[^/]"
            i += 1
        else:
            out += re.escape(pattern[i])
            i += 1
    return re.compile("^" + out + "$")


def filtered_workflows():
    """workflow name -> the regular expressions of its pull_request `paths:`."""
    found = {}
    for path in ci_yaml.workflows_in(WORKFLOWS):
        spec = ci_yaml.load(path)
        on = spec.get("on", spec.get(True))
        pr = on.get("pull_request") if isinstance(on, dict) else None
        if isinstance(pr, dict) and pr.get("paths"):
            found[os.path.basename(path)[:-4]] = [path_regex(p) for p in pr["paths"]]
    return found


def triggered(filters, f):
    names = sorted(n for n, regs in filters.items() if any(r.match(f) for r in regs))
    return ",".join(names) or "-"


def git(repo, *args):
    return subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True, check=True).stdout.strip()


def judge_repo(script, repo, base, head):
    out = os.path.join(repo, "..", "out.txt")
    open(out, "w").close()
    env = dict(os.environ, EVENT="pull_request", GITHUB_OUTPUT=out)
    subprocess.run(["bash", script, base, head], cwd=repo, env=env, capture_output=True)
    return dict(line.split("=", 1) for line in open(out).read().split())


def answers(files):
    """file -> (rust, build, debian, lint, vm)."""
    result = {}
    work = tempfile.mkdtemp()
    repo = os.path.join(work, "repo")
    os.makedirs(repo)
    git(repo, "init", "-q", "-b", "main")
    git(repo, "config", "user.email", "m@example.invalid")
    git(repo, "config", "user.name", "m")
    open(os.path.join(repo, "README.md"), "w").write("base\n")
    git(repo, "add", "-A")
    git(repo, "commit", "-q", "-m", "base")
    base = git(repo, "rev-parse", "HEAD")
    for f in files:
        git(repo, "reset", "-q", "--hard", base)
        git(repo, "clean", "-qfdx")
        path = os.path.join(repo, f)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        open(path, "w").write("changed\n")
        git(repo, "add", "-A")
        git(repo, "commit", "-q", "-m", "c")
        head = git(repo, "rev-parse", "HEAD")
        rust = judge_repo(os.path.join(CHECKS, "rust-ci-inputs.sh"), repo, base, head)
        if rust.get("run") == "false":
            r = "N"
        elif rust.get("docs_only") == "true":
            r = "D"
        elif rust.get("ci_only") == "true":
            r = "C"
        else:
            r = "R"
        deb = judge_repo(os.path.join(CHECKS, "debian-ci-inputs.sh"), repo, base, head)
        lint = judge_repo(os.path.join(CHECKS, "workflow-lint-inputs.sh"), repo, base, head)
        b = subprocess.run(["bash", os.path.join(CHECKS, "rust-build-inputs.sh"), f], capture_output=True).returncode
        v = subprocess.run(["bash", os.path.join(CHECKS, "podman-lane-paths.sh"), f], capture_output=True).returncode
        result[f] = (r, "N" if b == 0 else "B", "N" if deb.get("run") == "false" else "B",
                     "N" if lint.get("run") == "false" else "R", "N" if v == 0 else "V")
    return result


def trigger_rows(files, filters):
    rows = []
    for f in files:
        rows.append((f, triggered(filters, f)))
    return rows


def compress_triggers(rows):
    """A directory becomes `dir/*` when every file under it starts the same workflows."""
    def walk(prefix, items):
        answers_here = {a for _f, a in items}
        if prefix and len(answers_here) == 1:
            return [(prefix + "/*", next(iter(answers_here)))]
        out, groups = [], {}
        for f, a in items:
            rest = f[len(prefix) + 1:] if prefix else f
            groups.setdefault(rest.split("/")[0], []).append((f, a))
        for head, sub in sorted(groups.items()):
            path = f"{prefix}/{head}" if prefix else head
            if len(sub) == 1 and sub[0][0] == path:
                out.append((path, sub[0][1]))
            else:
                out.extend(walk(path, sub))
        return out
    return walk("", sorted(rows))


def load_triggers():
    rows = []
    for line in open(TRIGGERS):
        line = line.rstrip("\n")
        if line and not line.startswith("#"):
            pattern, want = line.split("\t")
            rows.append((pattern, want))
    return rows


def load():
    rows = []
    for line in open(EXPECTED):
        line = line.rstrip("\n")
        if not line or line.startswith("#"):
            continue
        pattern, want = line.split("\t")
        rows.append((pattern, tuple(want.split())))
    return rows


def lookup(rows, f):
    for pattern, want in rows:
        if fnmatch.fnmatchcase(f, pattern):
            return pattern, want
    return None, None


def compress(result):
    """Rows for --update: the highest directory under which every judged file has
    one answer becomes `dir/*`; a file whose directory is mixed stays a line of
    its own. Specific rows come before the directories that contain them."""
    def walk(prefix, items):
        answers_here = {a for _f, a in items}
        if prefix and len(answers_here) == 1:
            return [(prefix + "/*", next(iter(answers_here)))]
        rows, groups = [], {}
        for f, a in items:
            rest = f[len(prefix) + 1:] if prefix else f
            head = rest.split("/")[0]
            groups.setdefault(head, []).append((f, a))
        for head, sub in sorted(groups.items()):
            path = f"{prefix}/{head}" if prefix else head
            if len(sub) == 1 and sub[0][0] == path:
                rows.append((path, sub[0][1]))
            else:
                rows.extend(walk(path, sub))
        return rows
    return walk("", sorted(result.items()))


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else "--check"
    files = tracked()
    judged = sample(files)
    only = os.environ.get("MAP_ONLY")
    if only:  # the sabotage cases judge the few files their plant can move
        judged = [f for f in judged if f in only.split(",")] or only.split(",")
    result = answers(judged)
    if mode == "--update":
        with open(EXPECTED, "w") as fh:
            fh.write("# pattern <TAB> rust build debian lint vm  (see map.py; the first match decides)\n")
            for pattern, a in compress(result):
                fh.write(f"{pattern}\t{' '.join(a)}\n")
        with open(TRIGGERS, "w") as fh:
            fh.write("# pattern <TAB> the workflows with a paths: filter that start for it (see map.py; the first match decides)\n")
            for pattern, a in compress_triggers(trigger_rows(files, filtered_workflows())):
                fh.write(f"{pattern}\t{a}\n")
        print(f"expected.txt written: {len(compress(result))} patterns for {len(judged)} judged files")
        return 0
    rows = load()
    problems = []
    for f in files:
        pattern, _ = lookup(rows, f)
        if pattern is None:
            problems.append(f"{f}: no pattern covers it; say which suites it triggers")
    for f in judged:
        pattern, want = lookup(rows, f)
        if want is not None and tuple(want) != result[f]:
            problems.append(f"{f}: the judges answer {' '.join(result[f])}, the list holds {' '.join(want)} (pattern {pattern})")
    for pattern, _ in rows:
        if not any(fnmatch.fnmatchcase(f, pattern) for f in files):
            problems.append(f"{pattern}: matches no tracked file; remove it")
    trows = load_triggers()
    filters = filtered_workflows()
    for f in files:
        pattern, want = lookup(trows, f)
        if pattern is None:
            problems.append(f"{f}: no trigger pattern covers it")
        elif want != triggered(filters, f):
            problems.append(f"{f}: it starts {triggered(filters, f)}, the list holds {want} (pattern {pattern})")
    for pattern, _ in trows:
        if not any(fnmatch.fnmatchcase(f, pattern) for f in files):
            problems.append(f"{pattern}: matches no tracked file in triggers.txt; remove it")
    for p in problems:
        print(p)
    if not problems:
        print(f"ci-map: {len(files)} tracked files covered, {len(judged)} judged against {len(rows)} patterns, {len(trows)} trigger patterns")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
