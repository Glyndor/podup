#!/usr/bin/env python3
"""The value of every pin in the workflows and actions, checked on its own.

The wiring contract does not hold the commit an action is pinned to, because a
Dependabot pull request moves it and cannot regenerate a reference copy. That
leaves the value to this check, which every change under `.github/` has to pass:

  - every `uses`, `image` and `container` is read from the loaded YAML, never by
    a pattern over the lines, so what GitHub reads and this script does not is
    refused instead of being left unjudged;
  - an action is `<owner>/<repo>[/<path>]@<40 hex>` with the owner on a short
    list, followed on the same line by `# vMAJOR.MINOR.PATCH`;
  - that commit is exactly the commit the tag `vX.Y.Z` of that repository points
    to (the peeled one for an annotated tag), asked of the repository itself, and
    a repository that cannot be asked is a failure;
  - one (commit, version) pair per action in the whole tree, so the pin of a job
    that holds a token cannot drift from the others;
  - the version never goes below the one the base commit pins for that action;
  - an image is `<repository>[:<tag>]@sha256:<64 hex>` with the repository on a
    short list.

Run it as `python3 -I check-pins.py --head <tree> [--base <tree>]` from a copy that
the base commit owns, outside both trees. Exit 0 when every pin passes, 1 when one
does not, 2 when something could not be read or asked (also a failure).
"""
import argparse
import collections
import os
import re
import subprocess
import sys
import tempfile

# Next to this file when it runs from the copy the base commit owns, and in the tree
# otherwise.
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path[:0] = [HERE, os.path.join(HERE, "..", "lib")]
import yaml  # noqa: E402
import ci_yaml  # noqa: E402  (the loader that refuses a duplicate key)

OWNERS = {"actions", "github"}
IMAGES = {"debian"}

FULL_SHA = re.compile(r"[0-9a-f]{40}")
VERSION = re.compile(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")
LINE = re.compile(r"^\s*(?:-\s+)?uses:\s+(\S+)\s+#\s*(v(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*))\s*$")
IMAGE = re.compile(r"^([a-z0-9][a-z0-9._/-]*)(?::[A-Za-z0-9._-]+)?@sha256:[0-9a-f]{64}$")
IMAGE_ANYWHERE = re.compile(r"(?<![A-Za-z0-9._/-])([a-z0-9][a-z0-9._/-]*)(?::[A-Za-z0-9._-]+)?@sha256:[0-9a-f]{64}")


class Unreadable(Exception):
    """Something that has to be read or asked and could not be: a failure."""


def walk(node, found):
    """Every `uses`, `image` and `container` string of a loaded document."""
    if isinstance(node, dict):
        for key, value in node.items():
            if isinstance(value, str) and key in ("uses", "image", "container"):
                found.append((key, value))
            walk(value, found)
    elif isinstance(node, list):
        for value in node:
            walk(value, found)


def positions(node, found):
    """(line, value) of the value of every `uses` key in a composed document: where
    the reference is written, so a look-alike line elsewhere cannot stand in for it."""
    if isinstance(node, yaml.MappingNode):
        for key, value in node.value:
            if isinstance(key, yaml.ScalarNode) and key.value == "uses" and isinstance(value, yaml.ScalarNode):
                found.append((value.start_mark.line, value.value))
            positions(value, found)
    elif isinstance(node, yaml.SequenceNode):
        for value in node.value:
            positions(value, found)


def strings(node, found):
    if isinstance(node, dict):
        for value in node.values():
            strings(value, found)
    elif isinstance(node, list):
        for value in node:
            strings(value, found)
    elif isinstance(node, str):
        found.append(node)


def collect(root, problems):
    """(actions, images) found under `root`: action -> {(sha, version)} with the
    places that use it, image -> {digest string}. Problems are appended."""
    actions = collections.defaultdict(lambda: collections.defaultdict(list))
    images = collections.defaultdict(set)
    for rel in ci_yaml.files(root):
        path = os.path.join(root, rel)
        try:
            spec = ci_yaml.load(path)
            with open(path, newline="") as fh:
                text = fh.read()
            if re.search("\r(?!\n)|[\x85\u2028\u2029]", text):
                raise ValueError("a line break other than a line feed")  # lines would not be counted as YAML counts them
            lines = text.split("\n")
        except Exception as e:  # unreadable or malformed: GitHub may read it differently
            raise Unreadable(f"{rel}: cannot be read ({e})")
        refs = []
        walk(spec, refs)
        placed = []
        try:
            positions(yaml.compose(text, Loader=yaml.SafeLoader), placed)
        except yaml.YAMLError as e:
            raise Unreadable(f"{rel}: cannot be read ({e})")
        loaded = collections.Counter(v for k, v in refs if k == "uses")
        if collections.Counter(v for _l, v in placed) != loaded:
            problems.append(f"{rel}: the references of the loaded YAML and the ones found where they are written differ (an alias or an anchor?)")
            continue
        for line_no, ref in placed:
            if ref.startswith("./") or ref.startswith("docker://"):
                continue
            m = LINE.match(lines[line_no])
            if not m or m.group(1) != ref:
                problems.append(f"{rel}:{line_no + 1}: `{ref}` is not written as `uses: <ref> # vMAJOR.MINOR.PATCH` on one line")
                continue
            name, _, pin = ref.partition("@")
            parts = name.split("/")
            if len(parts) < 2 or not pin:
                problems.append(f"{rel}:{line_no + 1}: `{ref}` is not `<owner>/<repo>@<commit>`")
                continue
            if parts[0] not in OWNERS:
                problems.append(f"{rel}:{line_no + 1}: `{ref}` is owned by `{parts[0]}`, which is not one of {sorted(OWNERS)}")
            if not FULL_SHA.fullmatch(pin):
                problems.append(f"{rel}:{line_no + 1}: `{ref}` is not pinned to a full lower-case 40 character commit")
                continue
            actions[name][(pin, m.group(2))].append(rel)
        for key, value in refs:
            if key == "uses" and value.startswith("docker://"):
                value = value[len("docker://"):]
            elif key == "uses":
                continue
            if "${{" in value:
                continue  # the value comes from an input, which is scanned where it is written
            # The repository is judged below, with every other digest written anywhere in the file.
            if not IMAGE.match(value):
                problems.append(f"{rel}: image `{value}` is not `<repository>[:<tag>]@sha256:<digest>`")
        every = []
        strings(spec, every)
        for value in every:
            for m in IMAGE_ANYWHERE.finditer(value):
                if m.group(1) not in IMAGES:
                    problems.append(f"{rel}: image `{m.group(0)}` is from `{m.group(1)}`, which is not one of {sorted(IMAGES)}")
                images[m.group(0).split("@")[0]].add(m.group(0).split("@")[1])
    return actions, images


def version_tuple(version):
    return tuple(int(n) for n in VERSION.fullmatch(version).groups())


def tag_commit(repo, version):
    """The commit a tag of `<owner>/<repo>` points to, or raises Unreadable."""
    remote = os.environ.get("PINS_REMOTE", "https://github.com").rstrip("/")
    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": tempfile.gettempdir(),
        "GIT_CONFIG_GLOBAL": os.devnull,
        "GIT_CONFIG_SYSTEM": os.devnull,
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_TERMINAL_PROMPT": "0",
        "LC_ALL": "C",
    }
    last = ""
    for _attempt in range(3):
        # Outside every repository, so that no local configuration rewrites the address.
        with tempfile.TemporaryDirectory() as cwd:
            run = subprocess.run(
                ["git", "ls-remote", f"{remote}/{repo}", f"refs/tags/{version}", f"refs/tags/{version}^{{}}"],
                cwd=cwd, env=env, stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=90,
            )
        if run.returncode == 0:
            found = {}
            for row in run.stdout.splitlines():
                sha, _, ref = row.partition("\t")
                found[ref] = sha
            sha = found.get(f"refs/tags/{version}^{{}}") or found.get(f"refs/tags/{version}")
            if sha:
                return sha
            last = f"the repository has no tag {version}"
            break
        last = run.stderr.strip()[:200]
    raise Unreadable(f"{repo}: cannot find the commit of {version} ({last})")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--head", required=True)
    ap.add_argument("--base")
    args = ap.parse_args()
    problems = []
    try:
        head, head_images = collect(args.head, problems)
        base = {}
        if args.base:
            base, _ = collect(args.base, [])
        unreadable = []
        for name, pairs in sorted(head.items()):
            if len(pairs) > 1:
                shown = ", ".join(f"{sha[:12]} {ver}" for sha, ver in sorted(pairs))
                problems.append(f"`{name}` is pinned to more than one pair in the tree: {shown}")
        for tag, digests in sorted(head_images.items()):
            if len(digests) > 1:
                problems.append(f"image `{tag}` is pinned to more than one digest in the tree")
        for name, pairs in sorted(head.items()):
            floor = max((version_tuple(v) for _s, v in base.get(name, {})), default=None)
            for sha, version in sorted(pairs):
                if floor is not None and version_tuple(version) < floor:
                    problems.append(f"`{name}` goes down to {version}; the base pins {'.'.join(map(str, floor))} or later (a lower version needs an explicit exception)")
        asked = set()
        for name, pairs in sorted(head.items()):
            repo = "/".join(name.split("/")[:2])
            for sha, version in sorted(pairs):
                if (repo, version, sha) in asked:
                    continue
                asked.add((repo, version, sha))
                try:
                    want = tag_commit(repo, version)
                except (Unreadable, subprocess.TimeoutExpired, OSError) as e:
                    unreadable.append(str(e))
                    continue
                if want != sha:
                    problems.append(f"`{name}@{sha}` is not the commit of {version}: that tag is {want}")
    except Unreadable as e:
        print(f"check-pins: {e}")
        return 2
    for p in dict.fromkeys(problems + unreadable):
        print(f"check-pins: {p}")
    if unreadable and not problems:
        return 2
    if problems or unreadable:
        return 1
    print(f"check-pins: {sum(len(v) for v in head.values())} pin pair(s) for {len(head)} action(s) and {len(head_images)} image(s) are in order.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
