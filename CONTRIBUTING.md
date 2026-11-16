# Contributing to Glyndor/podup

Contributions are invitation-only. Bug reports and ideas through issues are welcome; unsolicited pull requests are not accepted. Invited contributors follow the branch flow and checks below.

## What this repository is

The internal Rust library supports tests; it is unpublished and has no stable API promise. Release CI uploads artifacts without committing. Dependabot changes require review.

## Building from source

From a checkout, with Rust 1.88 or newer and a C compiler and archiver (needed by `ring`):

```sh
cargo build --release
```

The binary is `target/release/podup`. The default build enables watch, completions and update; the Debian package leaves update out.

## Branch flow

- Branch from `develop` and target `develop`; `main` accepts the release PR from `develop`.
- Squash into `develop`; use a Conventional Commit PR title.
- Merge the release PR into `main` with a merge commit. Release tags must be reachable from `main`.

Mention the issue in the PR and mark its status done. It closes when the release PR merges into `main`.

## Before you open a pull request

- Open an issue and apply its `type:`, `prio:`, `effort:`, `status:` and `area:` labels.
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
| `ci.yml` | the rustfmt / clippy / test / coverage / MSRV gates |
| `repository-health.yml` | the advisory audits about the repository rather than about a change: the freshness of the scheduled workflows, the health of `main` and `develop`, and whether Dependabot has spoken recently. They run on protected-branch pushes, on a daily schedule and on demand, and never on a pull request |
| `lint-shell.yml` | shellcheck and the shell test suite |
| `lint-powershell.yml` | PSScriptAnalyzer on `install.ps1` |
| `debian-build.yml` | the `.deb` builds under `debian/rules`' narrower feature set, on every change that can shift it |
| `binary-budget.yml` | the release binary fits `bench/budgets/binary-budget-mib` |
| `asset-contract.yml` | the release asset names, signing-key rotation, install self-test, and shell/PowerShell signing fixtures |
| `podman-lane.yml` | the integration suite against live Podman 5 and Podman 6 in a Fedora qemu VM, with VM boot skipped for changes that cannot affect runtime behaviour (prose, templates) |
| `podman-lane-develop-nightly.yml` | the coverage lane against Podman 6, fired nightly from `develop` |
| `dco.yml`, `line-limit.yml`, `workflow-lint.yml` | one rule each |
| `main-guard.yml` | `develop-only`: only the release pull request can target `main` |
| `release.yml` | tag validity, Cargo.toml / debian/changelog / Cargo.lock agreement, `cargo audit`, signed binaries, GitHub Release |

Package and semver checks are disabled in `ci.yml`. Reusable workflows are local files under `.github/workflows/`.

Renaming a job changes its required-check name. Update the ruleset when renaming jobs.

### Running the Linux jobs on a self-hosted runner

Every Linux job takes its runner from one repository variable, `CI_RUNNER`, written as `runs-on: ${{ fromJSON(vars.CI_RUNNER || '"ubuntu-latest"') }}`. The inner quotes are needed for the default, which is a JSON string.

- **Turn it on:** Settings, Secrets and variables, Actions, Variables, new repository variable `CI_RUNNER`. A single label is written with its quotes, `"my-runner"`; several labels are a JSON array, `["self-hosted","linux","x64"]`.
- **Turn it off:** delete the variable. Nothing else changes, and every job is back on `ubuntu-latest`.
- **One label, no others:** to split the load, filter by workflow on the runner itself.
- **macOS and Windows jobs** stay on GitHub's runners, as does anything that takes its runner from `matrix.os`, `matrix.runner` or `inputs.runner` with a value other than `ubuntu-latest`.

Three jobs ignore the variable on purpose, and `tests/repository/workflows/workflow_linux_runner_variable.rs` lists them with the reason: `release.yml` (it builds, signs and publishes), and the `podman-vm` legs of `podman-lane.yml` and `podman-lane-develop-nightly.yml` (they boot QEMU with `/dev/kvm` and `sudo`, and `Supported Podman majors` is a required check). The same test fails when any other Linux `runs-on` is written as the fixed literal again.

**Security.** This is a public repository, and a self-hosted runner executes the code of a pull request from a fork. Require approval for outside collaborators (Settings, Actions, General, Fork pull request workflows), never run a self-hosted runner on a `pull_request_target` workflow, and make the runner ephemeral: a fresh container or VM for every job, so one job cannot leave anything for the next.

**What the runner needs.** These are the jobs that use something a bare runner may lack:

- `sudo` and `apt-get`: `linux-hardening.yml` (installs `musl-tools`), `reusable-shell-ci.yml`, and `reusable-rust-debian.yml` (inside its Debian container).
- A container runtime for `container:` jobs: `reusable-rust-debian.yml` and `pin-watch.yml`.
- Network access to the registries, crates.io and GitHub, and the toolchains the workflows install themselves (Rust, `cargo-audit`, `cargo-deny`, `cargo-llvm-cov`).

## Security

Report vulnerabilities privately through the repository's Security tab; the organization's security policy applies.