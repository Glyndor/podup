"""The checks of the rehearsal of a release: the call in release.yml and the reusable it calls.

Read by wiring.py, which hands it the job, the reusable, the steps of the publishing job and the
throwaway key of release.yml.
"""
import re

OVERLAY = "Take the release scripts from the commit that started the rehearsal"

# The call of the rehearsal, whole but for its name. The condition carries a status function and the results
# it waits for: `attest` is skipped in a rehearsal, and without one the job would be skipped with it.
REHEARSAL_CALL = {
    "if": "${{ !cancelled() && inputs.dry_run && needs.verify.result == 'success' && needs.seal.result == 'success' }}",
    "needs": ["verify", "seal"],
    "permissions": {"contents": "read"},
    "uses": "./.github/workflows/reusable-release-rehearsal.yml",
    "with": {"tag": "${{ needs.verify.outputs.tag }}", "commit": "${{ needs.verify.outputs.commit }}", "version": "${{ needs.verify.outputs.version }}",
             "sums": "${{ needs.seal.outputs.sums }}", "dry_run": "${{ github.event_name == 'workflow_dispatch' && inputs.dry_run }}"},
}


def uses(step, action):
    return action in str(step.get("uses", ""))


def shared_checks(steps):
    """The steps the publishing job and the rehearsal have in common, with the values the caller hands the
    reusable written as the publishing job reads them, so that the two can be compared step by step."""
    def same(value):
        if isinstance(value, dict):
            return {k: same(v) for k, v in value.items() if k != "REHEARSAL"}
        if isinstance(value, str):
            return value.replace("needs.verify.outputs.", "inputs.").replace("needs.seal.outputs.sums", "inputs.sums")
        return value

    kept = []
    for s in steps:
        text = str(s.get("run", ""))
        if uses(s, "actions/checkout") or s.get("name") == OVERLAY or str(s.get("name", "")).startswith("Say what a rehearsal"):
            continue
        if "verify-provenance.sh" in text or "gh release create" in text:
            continue
        kept.append(same(s))
    return kept


def rehearsal_problems(call, spec, release, seed):
    """The job that checks the set in a rehearsal: a call that can only read, and the checks of the publisher."""
    out = []
    if {k: v for k, v in call.items() if k != "name"} != REHEARSAL_CALL:
        out.append("`rehearsal` is not the call of the reusable of the rehearsal: only in a rehearsal, after `verify` and "
                   "`seal`, with a token that reads and with what the two named")
    if spec.get("permissions") != {"contents": "read"}:
        out.append("the reusable of the rehearsal does not hold `contents: read` and nothing else at the top")
    if (spec.get("env") or {}) != {"REHEARSAL_SEED_B64": seed} or not seed:
        out.append("the reusable of the rehearsal does not have the throwaway key of release.yml and nothing else in its environment")
    if list(spec["jobs"]) != ["rehearsal"]:
        out.append("the reusable of the rehearsal has a job other than `rehearsal`")
    job = spec["jobs"]["rehearsal"]
    if job.get("permissions") != {"contents": "read"}:
        out.append("the job of the reusable of the rehearsal can do more than read")
    if "environment" in job or "uses" in job:
        out.append("the job of the reusable of the rehearsal has an environment or calls another workflow")
    if job.get("if") != "${{ !cancelled() && inputs.dry_run }}" or job.get("continue-on-error"):
        out.append("the job of the reusable of the rehearsal does not run in a rehearsal and only there")
    if any(w in str(spec) for w in ("secrets", "github.token", "GH_TOKEN")):
        out.append("the reusable of the rehearsal reaches for a secret or for the token")
    steps = job.get("steps") or []
    if any(re.search(r"\bgh\s+[a-z]|--verify-tag", str(s.get("run", ""))) for s in steps):
        out.append("the rehearsal runs `gh`, which a rehearsal has no use for and a publication needs")
    if shared_checks(steps) != shared_checks(release):
        out.append("the checks of the rehearsal are not those of the publishing job, in the same order")
    checkout = next((s for s in steps if uses(s, "actions/checkout")), {})
    if {k: v for k, v in (checkout.get("with") or {}).items() if k != "ref"} != {"persist-credentials": False}:
        out.append("the checkout of the rehearsal keeps a credential or takes something else")
    signatures = next((s for s in steps if "verify-signatures.sh" in str(s.get("run", ""))), {})
    if (signatures.get("env") or {}).get("REHEARSAL") != "${{ inputs.dry_run }}":
        out.append("the rehearsal does not verify the signatures as those of a rehearsal")
    if not steps or not str(steps[-1].get("name", "")).startswith("Say what a rehearsal") or "if" in steps[-1]:
        out.append("a rehearsal does not say what it would have published")
    return out
