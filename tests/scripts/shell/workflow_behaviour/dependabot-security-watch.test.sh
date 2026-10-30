#!/usr/bin/env bash
# Behaviour tests for the Dependabot security watcher, driven through its CLI.
# Each rejection checks the diagnostic as well as the exit status. In a mixed
# listing both Dependabot spellings must appear; exit 1 alone can hide a miss.
set -uo pipefail
export LC_ALL=C

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
script="$HERE/.github/scripts/checks/dependabot_security_watch.py"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
pass=0
fail=0

check() { # <description> <expected> <actual>
	if [ "$2" = "$3" ]; then
		echo "ok    $1"
		pass=$((pass + 1))
	else
		echo "FAIL  $1"
		echo "        expected: $2"
		echo "        actual:   $3"
		fail=$((fail + 1))
	fi
}
said() { printf '%s\n' "$out" | grep -Fq -- "$1" && echo 1 || echo 0; }
run() {
	out="$(printf '%s\n' "$1" | python3 -I "$script" --now 2026-09-02T12:00:00Z 2>&1)"
	rc=$?
}

run '[]'
check "empty listing is accepted" 0 "$rc"
check "empty listing reports clean" 1 "$(said 'No Dependabot pull request is stuck')"

run '[{"number":101,"author":{"login":"maintainer"},"headRefName":"feature/x"}]'
check "human branch is accepted" 0 "$rc"
check "human PR is absent from the report" 0 "$(said '#101')"

listing='[
 {"number":104,"title":"bump rustls","author":{"login":"dependabot[bot]"},"headRefName":"dependabot/cargo/rustls","createdAt":"2026-08-20T09:00:00Z"},
 {"number":105,"author":{"login":"maintainer"},"headRefName":"develop"},
 {"number":106,"title":"bump ring","author":{"login":"app/dependabot"},"headRefName":"dependabot/cargo/ring","createdAt":"2026-08-20T09:00:00Z"}
]'
run "$listing"
check "two stuck PRs are rejected" 1 "$rc"
check "user-shaped Dependabot PR is reported with age and title" 1 "$(said '#104 stuck against main for 13 days: bump rustls')"
check "app-shaped Dependabot PR is reported with age and title" 1 "$(said '#106 stuck against main for 13 days: bump ring')"
check "human release PR is omitted" 0 "$(said '#105')"
check "report includes the recovery procedure" 1 "$(said 'Re-land each one by hand off develop')"

printf '%s\n' "$listing" > "$WORK/prs.json"
out="$(python3 -I "$script" --now 2026-09-02T12:00:00Z "$WORK/prs.json" 2>&1)"; rc=$?
check "file argument rejects the same stuck listing" 1 "$rc"
check "file argument includes the app PR" 1 "$(said '#106')"

for login in 'dependabot[bot]' 'app/dependabot'; do
	run "[{\"number\":102,\"author\":{\"login\":\"$login\"},\"headRefName\":\"develop\"}]"
	check "$login from develop is accepted" 0 "$rc"
	check "$login release PR is absent" 0 "$(said '#102')"
	run "[{\"number\":108,\"author\":\"$login\",\"headRefName\":\"dependabot/cargo/x\"}]"
	check "$login as a string is rejected" 1 "$rc"
	check "$login string author PR is reported despite missing date" 1 "$(said '#108 stuck against main for ? days')"
done

for payload in '{not json' '{"number":1}' '[{"author":{"login":"app/dependabot"}}]' '[null]'; do
	run "$payload"
	check "invalid listing exits 2: $payload" 2 "$rc"
	check "invalid listing names the read failure" 1 "$(said 'could not read the pull request listing:')"
	check "invalid listing never reports clean" 0 "$(said 'No Dependabot pull request is stuck')"
done

out="$(python3 -I "$script" "$WORK/missing.json" 2>&1)"; rc=$?
check "missing file exits 2" 2 "$rc"
check "missing file names the read failure" 1 "$(said 'could not read the pull request listing:')"
out="$(printf '[]' | python3 -I "$script" --now yesterday 2>&1)"; rc=$?
check "invalid clock exits 2" 2 "$rc"
check "invalid clock names the argument" 1 "$(said '--now is not an ISO 8601 instant:')"

echo
echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
