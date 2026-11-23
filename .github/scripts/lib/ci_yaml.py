"""The one way the CI checks of this repository read workflows and actions.

Two things that every validator needs and that went wrong when each wrote its
own:

  - `load` is a YAML loader that refuses a duplicate key. PyYAML's `safe_load`
    keeps the last of two equal keys and says nothing, so a validator that
    used it saw one `if:` and GitHub could be running the other.
  - `files` lists every workflow and every action: `.yml` and `.yaml` under
    `.github/workflows`, found recursively, and `action.yml` and `action.yaml`
    anywhere in the tree. What the list leaves out, a validator does not compare
    and GitHub still executes.

Import it from the script's own directory (`sys.path[0]` is that directory when
a script runs).
"""
import os
import re

import yaml

WORKFLOW_DIR = os.path.join(".github", "workflows")
SKIP_DIRS = {".git", "target", "node_modules"}


class StrictLoader(yaml.SafeLoader):
    """SafeLoader that raises on a duplicate mapping key."""


def _mapping(loader, node, deep=False):
    seen = set()
    for key_node, _value in node.value:
        key = loader.construct_object(key_node, deep=deep)
        if key in seen:
            raise yaml.constructor.ConstructorError(
                None, None, f"duplicate key {key!r}", key_node.start_mark
            )
        seen.add(key)
    return yaml.SafeLoader.construct_mapping(loader, node, deep)


StrictLoader.add_constructor(yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, _mapping)


def check_name(name):
    """The name a job reports before the part that depends on the run: a job name
    may end in an expression (`... (promotion)` for a pull request into main, the
    event for a push), and the name a ruleset requires is what comes before it."""
    return re.sub(r"\s*\$\{\{.*\}\}\s*$", "", name) if isinstance(name, str) else name


SHA_REF = re.compile(r"@[0-9a-f]{40}$")
IMAGE_DIGEST = re.compile(r"@sha256:[0-9a-f]{64}")


def normalize_pins(value, key=None):
    """The value with the pins an automatic update moves replaced by a marker.

    A Dependabot pull request moves the commit an action is pinned to (and the digest
    of an image) and changes nothing else, so a contract that held the value would
    be red on every one of them for good. What is kept is what an update does not
    move: the name of the action, that it is pinned and that the pin is a full 40
    character commit (a tag, a branch, a short hash or no pin at all stays as it is
    and so differs), and for an image that it is pinned by a sha256 digest.
    """
    if isinstance(value, dict):
        return {k: normalize_pins(v, k) for k, v in value.items()}
    if isinstance(value, list):
        return [normalize_pins(v, key) for v in value]
    if isinstance(value, str):
        if key == "uses":
            value = SHA_REF.sub("@<commit>", value)
        return IMAGE_DIGEST.sub("@sha256:<digest>", value)
    return value


def load(path):
    """The parsed file, or raises yaml.YAMLError (a duplicate key is one)."""
    with open(path) as fh:
        return yaml.load(fh, Loader=StrictLoader)


def workflows_in(directory):
    """Every `.yml` and `.yaml` under `directory`, recursively, sorted, as given."""
    found = []
    for dirpath, _dirnames, names in os.walk(directory):
        found.extend(os.path.join(dirpath, n) for n in names if n.endswith((".yml", ".yaml")))
    return sorted(found)


def files(root="."):
    """Every workflow and every action under `root`, sorted, as paths relative to `root`."""
    found = set()
    found.update(workflows_in(os.path.join(root, WORKFLOW_DIR)))
    for dirpath, dirnames, names in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
        for name in names:
            if name in ("action.yml", "action.yaml"):
                found.add(os.path.join(dirpath, name))
    return sorted(os.path.relpath(f, root) for f in found)
