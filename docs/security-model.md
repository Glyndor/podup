# Security model

This document describes podup's privilege posture, trust boundaries, and attack
surface so operators can reason about it during a security review (for example
an ATO/SSP assessment). The self-update and release trust chain is covered
separately in [self-update.md](self-update.md).

## Privilege posture

- podup runs entirely as the **invoking user**. It is not setuid/setgid and
  acquires no capabilities of its own.
- It drives **rootless Podman** over the libpod REST API on a Unix socket. Any
  privilege a container ends up with is granted by Podman/the kernel, bounded by
  the launching user's own privileges — a rootless container can never exceed
  them. Fields that assume more (`privileged`, `oom_kill_disable`,
  `mem_swappiness`, `cpu_rt_*`) are forwarded but warned about, since they are
  reduced or ineffective rootless.
- podup keeps **no persistent state** of its own outside the Podman objects it
  creates and a per-project advisory lock file under the user's runtime
  directory.

## Trust boundaries

| Boundary | Trusted? | Notes |
|----------|----------|-------|
| Podman socket (`PODMAN_SOCKET`/`DOCKER_HOST`) | Trusted | Whoever can reach it controls the engine; this is the primary boundary. |
| Compose file and its referenced files | **Trusted input** | Treated like a Makefile (see below). |
| Release artifacts (`podup update`, installer) | Untrusted transport | Verified against an embedded Ed25519 key + provenance attestation, fail-closed. |
| Container filesystem (e.g. `cp` archives) | Untrusted | Tar extraction refuses path-traversal (zip-slip) entries. |
| Network/TLS to GitHub/crates.io | Untrusted | Integrity comes from signatures, not transport. |

## Compose files are trusted input

A compose file is treated like a Makefile: running podup on one is equivalent to
trusting its author. Path-valued keys the spec resolves relative to the compose
file (`extends.file`, `env_file`, `label_file`, `include`) may reference paths
outside the project directory, including `../`. Do **not** run podup on a compose
file from an untrusted source. (`include` still rejects absolute paths as
non-portable, but this is hardening, not a security guarantee.)

## Secret and config handling

- `secrets:`/`configs:` sourced from inline `content:` or `environment:` are
  created as Podman-native secrets over the libpod API (under a project-scoped
  name) and injected into the container — podup writes no secret material to a
  host directory. They are removed again on `podup down`.
- `external: true` secrets/configs are injected as Podman-native secrets
  (pre-flighted for existence), pointing at a pre-existing `podman secret`.
- `file:` secrets/configs are bind-mounted read-only from the host path you
  provide; the file already lives on the host, so no copy is made.
- Dangerous secret file modes (setuid/setgid/sticky/executable) are rejected.
- The `config` subcommand redacts inline `content:` secrets before printing.

## Logging and information disclosure

- Default logging does not print secret values. Running with `RUST_LOG=debug`
  can emit environment variable values and resolved secret/config file paths;
  avoid debug logging where the terminal or log sink is not trusted.
- podup writes no secret material to its own persistent state.

## Supply chain

- Dependencies are pinned in `Cargo.lock`; `cargo deny` enforces a license
  allowlist and bans yanked crates, and `cargo audit` runs weekly in CI.
- No third-party CI actions are used — only GitHub-owned (SHA-pinned) actions.
- Releases are Ed25519-signed and carry GitHub build-provenance attestations; a
  CycloneDX SBOM and third-party license attribution are published with each
  release. See [self-update.md](self-update.md) for verification steps.
- The Debian package can be built fully offline from a vendored crate tree, for
  air-gapped/classified environments.

## Memory safety

The crate forbids `unsafe` by default (`#![deny(unsafe_code)]`). The few
unavoidable FFI calls (rootless uid/gid lookups, `flock`, `stat`) are isolated,
individually justified with safety comments, and unit-tested.

## Reporting

Report vulnerabilities privately via the repository's **Security tab → Report a
vulnerability** (never a public issue). See the organization
[security policy](https://github.com/Glyndor/.github/blob/main/SECURITY.md) for
response targets.
