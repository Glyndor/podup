# Contributing to Glyndor/podup

Contributions are invitation-only. Bug reports and ideas through issues are welcome; unsolicited pull requests are not accepted. Invited contributors follow the branch flow and checks below.

## What this repository is

The internal Rust library supports tests; it is unpublished and has no stable API promise. Release CI uploads artifacts without committing. Dependabot changes require review.

## Building from source

A C compiler and archiver are needed for ring. The default build enables watch, completions and update; the result is `target/release/podup`. The Debian package excludes update.

## Branch flow

- Branch from `develop` and target `develop`; `main` accepts the release PR from `develop`.
- Squash into `develop`; use a Conventional Commit PR title.
- Merge the release PR into `main` with a merge commit. Release tags must be reachable from `main`.

Mention the issue in the PR and mark its status done. It closes when the release PR merges into `main`.

## Before you open a pull request

- Open an issue and apply `type`, `priority`, `effort`, `status` and `area` labels.
- Sign commits and add DCO sign-off with `git commit -s`.
- Use a Conventional Commit PR title.

## Tests

Run the suite before pushing. A failure must surface as a non-zero exit, so wrap the block in a subshell that ends with `exit "$fail"`:

```sh
(
  fail=0
  cargo fmt --all --check || fail=1
  cargo clippy --locked --all-targets --all-features -- -D warnings || fail=1
  cargo test --locked --all-features --workspace || fail=1
  for t in tests/shell/*.test.sh; do bash "$t" || { echo "FAILED: $t"; fail=1; }; done
  find . -type f \( -name '*.sh' -o -name '*.bash' \) -not -path './.git/*' \
    -print0 | xargs -0 -r shellcheck --severity=style || fail=1
  exit "$fail"
)
```

CI toolchain and MSRV pins live in `reusable-rust-ci.yml` and `ci.yml`. `tests/workflow_toolchain_pin` checks agreement; `pin-watch` checks stable updates weekly.

The local `shellcheck` command scans `.sh`/`.bash` files; CI also includes extensionless scripts with shell shebangs.

A shell test no workflow runs is indistinguishable from one that passes. `tests/shell/ci-runs-every-test.test.sh` checks both missing and nonexistent test paths. Rust tests are discovered by Cargo. Verify a test fails when its control is removed, and assert the expected diagnostic as well as the exit status.

## Workflows

CI is split by responsibility rather than gathered in one file:

| file | what fails there |
|---|---|
| `ci.yml` | the rustfmt / clippy / test / coverage / MSRV gates, plus freshness audits on the scheduled workflows |
| `lint-shell.yml` | shellcheck and the shell test suite |
| `lint-powershell.yml` | PSScriptAnalyzer on `install.ps1` |
| `debian-build.yml` | the `.deb` builds under `debian/rules`' narrower feature set, on every change that can shift it |
| `binary-budget.yml` | the release binary fits `bench/binary-budget-mib` |
| `asset-contract.yml` | the release asset names, signing-key rotation, install self-test, and shell/PowerShell signing fixtures |
| `podman-lane.yml` | the integration suite against live Podman 5 and Podman 6 in a Fedora qemu VM, with VM boot skipped for changes that cannot affect runtime behaviour (prose, templates) |
| `podman-lane-develop-nightly.yml` | the coverage lane against Podman 6, fired nightly from `develop` |
| `dco.yml`, `line-limit.yml`, `workflow-lint.yml` | one rule each |
| `main-guard.yml` | `develop-only`: only the release pull request can target `main` |
| `release.yml` | tag validity, Cargo.toml / debian/changelog / Cargo.lock agreement, `cargo audit`, signed binaries, GitHub Release |

Package and semver checks are disabled in `ci.yml`. Reusable workflows are local files under `.github/workflows/`.

Renaming a job changes its required-check name. Update the ruleset when renaming jobs.

## Security

Report vulnerabilities privately through the repository's Security tab; the organization's security policy applies.