# Contributing to Glyndor/podup

Contributions are invitation-only. Bug reports and ideas through issues are welcome; unsolicited pull requests are not accepted. Invited contributors follow the branch flow and checks below.

## What this repository is

The internal Rust library supports tests; it is unpublished and has no stable API promise.

## Building from source

From a checkout, with Rust 1.88 or newer and a C compiler and archiver (needed by `ring`):

```sh
cargo build --release
```

The binary is `target/release/podup`. The default build enables watch, completions and update; the Debian package leaves update out.

## Branch flow

- Branch from `develop` and target `develop`; `main` accepts the release PR from `develop`.
- Squash into `develop`; use a Conventional Commit PR title.
- Editing the title or the body of a pull request does not run the CI again, and neither does reopening it: a second set of runs of the same commit would cancel the first and leave a red check under the required names. Changing the base needs a push. `develop` is reached only through the merge queue, which runs everything again on the result that lands, so a base that changed after the checks ran is judged there. Only `guard`, which judges the base itself, runs again on an edit.

Mention the issue in the PR and mark its status done. It closes when the release PR merges into `main`.

## Before you open a pull request

- Open an issue and apply its `type:`, `prio:`, `effort:`, `status:` and `area:` labels.
- Sign commits and add DCO sign-off with `git commit -s`, using your GitHub noreply address (`git config user.email`). `commit hygiene` rejects any other address in the author, `Signed-off-by` or `Co-authored-by`, and a line that looks like a credential.
- Use a Conventional Commit PR title.

## Tests

Run the suite before pushing. A failure must surface as a non-zero exit, so wrap the block in a subshell that ends with `exit "$fail"`:

```sh
(
  fail=0
  cargo fmt --all --check || fail=1
  cargo clippy --locked --all-targets --all-features -- -D warnings || fail=1
  cargo test --locked --all-features --workspace || fail=1
  for t in tests/scripts/shell/*/*.test.sh; do bash "$t" || { echo "FAILED: $t"; fail=1; }; done
  find . -type f \( -name '*.sh' -o -name '*.bash' \) -not -path './.git/*' \
    -print0 | xargs -0 -r shellcheck --severity=style || fail=1
  exit "$fail"
)
```

CI toolchain and MSRV pins live in `reusable-rust-ci.yml` and `ci.yml`. `tests/repository/workflows/workflow_toolchain_pin` checks agreement; `pin-watch` checks stable updates weekly.

The local `shellcheck` command scans `.sh`/`.bash` files; CI also includes extensionless scripts with shell shebangs.

A shell test no workflow runs is indistinguishable from one that passes. `tests/scripts/shell/suite_guards/ci-runs-every-test.test.sh` checks both missing and nonexistent test paths. Rust tests are discovered by Cargo. Verify a test fails when its control is removed, and assert the expected diagnostic as well as the exit status.

## Workflows

CI is split by responsibility rather than gathered in one file:

| file | what fails there |
|---|---|
| `ci.yml` | one required check, `rust / Gate`, which reads the result of every rustfmt / clippy / test / coverage / MSRV / doc suite and certifies each skip; the suites report their own checks but do not block on their own |
| `repository-health.yml` | the advisory audits about the repository rather than about a change: the freshness of the scheduled workflows, the health of `main` and `develop`, and whether Dependabot has spoken recently. They run on protected-branch pushes, on a daily schedule and on demand, and never on a pull request |
| `lint-shell.yml` | shellcheck and the shell test suite |
| `lint-powershell.yml` | PSScriptAnalyzer on `install.ps1` |
| `debian-build.yml` | the `.deb` builds under `debian/rules`' narrower feature set, on every change that can shift it |
| `binary-budget.yml` | the release binary fits `bench/budgets/binary-budget-mib` |
| `asset-contract.yml` | the release asset names, install self-test, and shell/PowerShell signing fixtures |
| `podman-lane.yml` | the integration suite against live Podman 6 in a Fedora rawhide qemu VM, with VM boot skipped for changes that cannot affect runtime behaviour (prose, templates) |
| `podman-lane-develop-nightly.yml` | the coverage lane against Podman 6, fired nightly from `develop` |
| `dco.yml`, `line-limit.yml`, `workflow-lint.yml` | one rule each |
| `main-guard.yml` | `develop-only`: only the release pull request can target `main` |
| `release.yml` | the release checks, signed binaries, GitHub Release |

Package and semver checks are disabled in `ci.yml`. Reusable workflows are local files under `.github/workflows/`.

## Security

Report vulnerabilities privately through the repository's Security tab; the organization's security policy applies.