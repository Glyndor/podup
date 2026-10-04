# Threat model

Threats, controls and evidence; [security-model.md](security-model.md) describes runtime boundaries. Evidence is a repository test, workflow gate or dated observation. Missing evidence and residual risks are explicit.

Evidence is of three kinds, and the column says which: a **test** is a case in the repository that fails when the control is removed; a **gate** is a required status check that blocks a merge; a **measurement** is a number read off a real run, with the date.

## Assets

| Asset | Where it lives | Why it matters |
|---|---|---|
| The operator's Podman engine | the libpod socket, local to the machine | whoever drives it runs containers as the operator |
| Compose files and the files they reference | the operator's project directory | podup executes what they describe, as a Makefile would |
| Secrets and configs | Podman-native secrets, created per project | injected into containers; never written to a host directory by podup |
| The release binaries and packages | GitHub releases, the apt archive, the Homebrew tap, the Scoop bucket | what every install and every self-update runs |
| The release signing key | the organization's CI secret; public half embedded in the binary, the installers and the channels | the trust anchor for everything above |
| The container's writable layer | Podman storage | destroyed on a recreate; the operator must be told when that happens |

## Adversaries considered

- A network position between the operator and GitHub or a CDN: can serve any bytes, including an older, legitimately signed release.
- A compromised upstream crate or tool pulled into a build or a release job.
- A compromised release of a sibling product sharing the organization's release key.
- A hostile or buggy libpod answering the socket with unexpected shapes.
- A hostile compose file the operator chose to run.
- A local process on the same machine without the operator's privileges.

Out of scope: compromise of the operator account, the engine socket or the release signing key, and physical access.

## Threats and controls

### Supply chain of what the operator installs

| Threat | Control | Evidence |
|---|---|---|
| A release asset replaced on the wire or on a mirror | every published asset is Ed25519-signed; binary consumers verify signed manifest plus selected digest before installation; release CI also verifies every detached signature | test: `tests/fixtures/releases/` drives both installers through a deterministic three-slot fixture in `asset-contract.yml`; gate: `asset-contract` on every pull request |
| `SHA256SUMS` re-signed to match one swapped binary | release CI verifies every detached signature against consumer keys | gate: the release workflow step "Verify every signature against the keys consumers embed" |
| A CDN hands back an older, legitimately signed release (rollback) | Consumers verify signed manifest plus selected digest before installation; CI verifies detached asset signatures. Tag/bytes mismatches fail. The updater tests after replacement and attempts rollback; it rejects non-newer versions unless forced. Coherent old tag/bytes can pass installer consistency and suppress discovery. | test: `install.sh:verify_version_self_test`, `install.ps1:Test-StagedVersion`; gate: `asset-contract` exercises the rotation slot |
| The embedded public key and the signing key drift apart | a release-time check verifies a real signature against the constant the installers ship; a regression test pins the embedded key against a real published `SHA256SUMS` | test: `embedded_key_verifies_real_release`; gate: `verify-signing-key.py` in the release workflow |
| A signing-key rotation strands installed clients | two key slots, make-before-break: a transition release trusts both keys and is signed with the old one, the next is signed with the new one | test: the three-slot fixture above exercises the rotation slot; the procedure is in [self-update.md](self-update.md) |
| A build script in a dependency runs beside the signing key | the jobs that hold the key install Python by hash only (`pip install --require-hashes`); a job that installs third-party tooling any other way must hold no secret | gate: `workflow-lint`'s tooling-isolation assertion, on every pull request |
| A vulnerable or yanked crate ships | `cargo audit` and `cargo deny` on every lockfile change and weekly; the release refuses to build on a finding. The root workspace lockfile is audited; the `fuzz` workspace is excluded. | gate: `audit / cargo audit`, `audit / cargo deny`; measurement: the weekly schedule is watched by `freshness-audit`, which fails when the cron stops |
| A third-party GitHub Action changes under a tag | no third-party actions; the runner primitives used are pinned to a commit SHA with the version beside it | gate: `workflow-lint` (actionlint plus the organization's rules) |
| A dependency bump goes unreviewed | Dependabot proposes every bump; a silent Dependabot is detected, and the detector distinguishes "nothing to bump" from "dead" by asking upstream for newer tags | gate: `dependabot-freshness` on every pull request |
| The Debian package built in a moving base image acquires a glibc floor nobody chose | the Linux release binaries are static musl; the `.deb` follows the same target | measurement: `static-pie linked`, zero `GLIBC_*` symbols on the published binary, 2026-08-30 |
| A Linux binary ships without the hardening its target is assumed to give it | Linux/deb binaries must pass static PIE, full RELRO, NX-stack and stripping checks before signing. Tests remove one property per control; PR builds run the same checks. | test: `tests/shell/check-hardening.test.sh`, one control binary per property |
| A Windows binary ships without the hardening its target is assumed to give it | Windows binaries must have `DYNAMIC_BASE`, `HIGH_ENTROPY_VA`, `NX_COMPAT` and `GUARD_CF` before signing. PE-header tests clear one bit per control. | test: `tests/powershell/check-hardening.Tests.ps1`, one control binary per property |
| A macOS binary ships without the hardening its target is assumed to give it | macOS binaries must be PIE and stripped before signing; tests disable one property at a time. | test: `tests/shell/check-hardening-macos.test.sh` |
| An image is replaced at the registry, or pulled from a registry nobody vetted | libpod applies the host image-signature policy during pulls; cached images under `missing` bypass a new policy check. Configuration/example lives once in security-model.md. | measurement: `Podman 5.7.0 reject-policy test`, 2026-09-03 |

### The engine boundary

| Threat | Control | Evidence |
|---|---|---|
| podup is pointed at a remote engine and secrets leave the machine | only `unix://` and `npipe://` are accepted, from every source of the socket path; remote schemes are rejected before a connection | test: unit tests on the socket resolver |
| A rootful engine is selected by accident | the auto-detected socket for uid 0 is the rootful one; an explicitly selected local socket path is accepted as given. The boundary is "local Unix socket" or "local named pipe", not "rootless engine" | test: socket-resolver tests for the rootful path; the rootless invariant itself is a configured prerequisite, not an enforced check |
| A project in a pod puts every service in one network namespace, so a compromised service reaches its siblings on `localhost` | pod mode is opt-in (`x-podman-pod: true`), only the network namespace is shared, and the doc names the consequence | test: the refusals and the pod request are unit-tested against the fake engine |
| libpod returns a name or path that reaches the filesystem | object names are validated against Podman's own pattern before use; project names are filtered at the dispatch boundary; Quadlet values are escaped and the unit filename re-checked before writing | test: `project_name_safety.rs`, `quadlet.rs`, the `names` and `unit` unit tests |
| A container archive escapes its destination on `cp` | tar extraction refuses path-traversal entries; the destination entry is compared against what was uploaded | test: `cp_flags.rs`, `copy` unit tests |
| libpod reports a failed pull on a `200` and podup starts a stale image | in-band `error` lines are read and surfaced; presence is verified after a pull | test: `pull` unit tests, `pull_ignore_failures_continues_past_bad_image` |
| A recreate destroys a writable layer without telling the operator | a replaced container is reported as `Recreating`/`Recreated`, never as `Starting` | test: `recreate_vocabulary.rs`, `recreate_on_image.rs` |

### Secrets and configs

| Threat | Control | Evidence |
|---|---|---|
| A secret is written to the host | every source, including `file:`, becomes a Podman-native secret; nothing is mounted from or written to a host directory | test: `secret_safety.rs`, the `secrets` unit tests |
| A secret value appears in an error or a log | the payload type has no `Display` and no `Debug` that prints it; `config` redacts inline content | test: `secret_safety.rs` |
| A build secret is baked into an image layer | build secrets are excluded from the build context in both context-tar builders | test: the build context unit tests |
| A `file:` secret with a dangerous mode is injected | setuid, setgid, sticky and executable modes are rejected | test: the `secrets/plan` unit tests |

### The host

| Threat | Control | Evidence |
|---|---|---|
| A systemd unit written by `autostart` injects a directive through a path | control characters are rejected in every value that lands in a unit; the unit stem is sanitised and re-checked before write | test: `autostart` unit tests |
| Two podup invocations race on one project | a per-project advisory lock at `$XDG_RUNTIME_DIR/podup` when that path is set and valid, otherwise the platform temp directory. The lock is `flock`-based and Unix-only; on Windows no OS lock is held and two concurrent invocations on the same project can race | test: `lock` unit tests |
| A staged self-update is swapped through a symlink | the staging file is created `O_EXCL` and opened `O_NOFOLLOW`; the target is replaced through an ordinary `rename` with its mode preserved | test: `install_binary` unit tests |
| Debug logging leaks environment values | documented: `RUST_LOG=debug` can print them; default logging does not | no test; the boundary is stated in security-model |

## How the evidence is kept honest

Reading a test does not say whether it works. The controls above were, where marked as tests, checked by deleting the control and watching the test go red; several tests in this repository were rewritten after they stayed green with their control gone (#1514 is one).

Tests should fail when their control is removed. The live lane runs rootless integration tests per supported Podman major for runtime-relevant changes; other PRs skip VM boot. Coverage excludes test bodies; its threshold is configured in CI.

## Residual risks

- One maintainer provides no independent review. Workflow writers can replace a required check under the same name; workflow changes need independent review when another maintainer joins.
- The organization shares its release key across products. A direct binary signature authenticates the signer, not this repository; each package channel must enforce its own product binding.
- The cryptographic implementations are not validated modules; deployments requiring one need a validated verification path.
- No independent security audit is recorded.
- The live lane runs one thread per Podman major to reduce VM transport failures.

## Reporting

Report vulnerabilities privately through the repository's **Security tab**. The organization's [security policy](https://github.com/Glyndor/.github/blob/main/SECURITY.md) carries the response targets.