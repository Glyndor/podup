# Security model

Runtime privileges, trust boundaries and validation. The [threat model](threat-model.md) maps controls to threats and residual risks. The release trust chain lives in [self-update.md](self-update.md).

## Privilege posture

- podup runs as the invoking user with no capabilities of its own. Engine socket access grants that engine's privileges.
- Rootless operation is a prerequisite, not enforced: uid 0 auto-selects the rootful socket and explicit local paths are accepted. Requested privilege settings are forwarded with warnings.
- podup keeps no persistent state of its own outside the Podman objects it creates and a per-project advisory lock file. Unix locks use a checked absolute `XDG_RUNTIME_DIR/podup`, or `temp_dir()/podup-<euid>` if unset/relative; unsafe absolute paths fail. Windows uses `%TEMP%/podup` without an OS lock. `autostart install` and explicit Quadlet export write requested host files.

## Trust boundaries

| Boundary | Trusted? | Notes |
|----------|----------|-------|
| Podman socket (`PODMAN_SOCKET`/`DOCKER_HOST`) | Trusted, local-only | Whoever can reach it controls the engine; this is the primary boundary. Only `unix://`/`npipe://` are accepted from `--socket`, `PODMAN_SOCKET`, `DOCKER_HOST`, or auto-detection; remote schemes are rejected fail-closed. |
| Compose file and its referenced files | **Trusted input** | Treated like a Makefile (see below). |
| Release artifacts (`podup update`, installer) | Untrusted transport | Verified against an embedded Ed25519 key. Per-asset `.sig` checks run in the release workflow and (via apt keyring) at install. Provenance attestation in the shell installers is defence-in-depth and only runs when a `gh` capable of verifying is present. |
| Container filesystem (e.g. `cp` archives) | Untrusted | Tar extraction refuses path-traversal (zip-slip) entries; the host-side `cp` destination is also walked component by component and any symlink at any component of the path is refused. |
| Network/TLS to GitHub/crates.io | Untrusted | Integrity comes from signatures, not transport. |

## Compose files are trusted input

A compose file is treated like a Makefile: running podup on one is equivalent to trusting its author. Path-valued keys the spec resolves relative to the compose file (`extends.file`, `env_file`, `label_file`, `include`) may reference paths outside the project directory, including `../`. Do not run podup on a compose file from an untrusted source. `include` accepts an absolute path and uses it as given; there is no containment to rely on.

## Container hardening (compose security keys)

The compose keys that constrain a container are translated onto Podman's `SpecGenerator` and take effect on the running container. Everything below remains bounded by the rootless ceiling: a key can only tighten, never widen, what the launching user already has.

A `podup audit` subcommand reads the project the same way `config` does and prints, for each service, which of those keys the file did not set. See [commands.md](commands.md#audit) for checks and exit codes; `audit` never changes runtime behavior, so it can be added to a CI gate.

- `security_opt` is parsed into the matching SpecGenerator fields:
  - `no-new-privileges` → `no_new_privileges`
  - `seccomp=<profile.json>` (and `seccomp=unconfined`) → `seccomp_profile_path`
  - `apparmor=<profile>` → `apparmor_profile`
  - `label=<opt>` (SELinux user/role/type/level, or `label=disable`) → `selinux_opts`
  - `mask=<paths>` / `unmask=<paths>` → `mask` / `unmask`
- `device_cgroup_rules` entries are parsed and applied as the container's device cgroup rules (a malformed entry is warned about and skipped, not fatal).
- CDI devices (Container Device Interface, e.g. `nvidia.com/gpu=all`) requested under `devices:` are passed through to Podman, which resolves them by name.
- Per-mount hardening (`noexec`, `nosuid` and `nodev`) is carried onto a volume's mount options. The short form spells them as raw mount options (`cache:/app/cache:noexec`); the long form takes them as booleans under `volume:`. See [Per-mount hardening options](docker-migration.md#per-mount-hardening-options-noexec-nosuid-nodev).

## Secret and config handling

- `secrets:`/`configs:` sourced from inline `content:` or `environment:`, and from a `file:` path, are created as Podman-native secrets over the libpod API (under a project-scoped name) and injected into the container; podup writes no secret material to a host directory. They persist until replaced on a later `up`, and are best-effort removed on `podup down`.
- `external: true` secrets/configs are injected as Podman-native secrets (pre-flighted for existence), pointing at a pre-existing `podman secret`.
- A `file:` source is read at `up` time and its bytes become the secret. With no `mode:` given the secret is mounted with the host file's own permission bits.
- File bytes are copied at `up`. Editing the source does not update a running container; recreate it after rotation.
- Dangerous secret file modes (setuid/setgid/sticky/executable) are rejected.
- The `config` subcommand redacts inline `content:` secrets before printing.

## Logging and information disclosure

- Default logging omits secret values. `RUST_LOG=debug` may expose environment values and resolved paths; use a trusted log sink.
- podup writes no secret material to its own persistent state.

## Memory safety

The crate forbids `unsafe` by default (`#![deny(unsafe_code)]`). The few unavoidable FFI calls (rootless uid/gid lookups, `flock`, `stat`) are isolated, individually justified with safety comments, and unit-tested.

## Supply chain

- Dependencies are pinned in `Cargo.lock`; `cargo deny` enforces a license allowlist and bans yanked crates, and `cargo audit` runs weekly in CI.
- No third-party CI actions are used, only GitHub-owned (SHA-pinned) actions.
- Releases are Ed25519-signed and carry GitHub build-provenance attestations; a CycloneDX SBOM and third-party license attribution are published with each release. Verification steps are in self-update.md.
- Release CI verifies Linux, Windows and macOS hardening before signing. Detailed gates live in threat-model.md; offline packaging stays in the Debian packaging and self-update guides.
- The Debian package can be built fully offline from a vendored crate tree, for air-gapped/classified environments.
- **Image signatures are the host's policy, and podup inherits it.** Every pull podup asks for is performed by libpod, which applies the host's image signature policy (system-wide in `/etc/containers/`, per user in `~/.config/containers/`) and the registry configuration under `registries.d`. The on-disk filename and the JSON shape are libpod configuration; podup does not read them directly. For services without `build:`, `x-podman-autoupdate: registry` pulls on `up` unless `--pull` overrides it. Host signature policy applies to those pulls. Measured on Podman 5.7.0 with a `reject` rule scoped to one repository, 2026-09-03: `up` fails at the pull with libpod's own message, `Source image rejected: Running image docker://... is rejected by policy.`, and creates no container. An example rule (replace placeholders with the host's real values):

```json
{
  "default": [{ "type": "reject" }],
  "transports": {
    "docker": {
      "registry.example.com": [{
        "type": "sigstoreSigned",
        "keyPath": "/etc/containers/keys/registry.example.com.pub",
        "signedIdentity": { "type": "matchRepository" }
      }]
    }
  }
}
```

## Self-update

Release verification, key rotation, installer errors and offline checks are in self-update.md.

## What podup validates before calling libpod

podup validates selected fields before calling libpod; other fields rely on libpod validation.

- Names: Podman object-name pattern; project names have stricter lowercase/length checks.
- URL segments: encoded before requests.
- Quadlet values: escaped, arguments quoted and output filenames checked.
- Signals: resolved to numeric libpod values.
- Pull policy: unknown values rejected.
- Shutdown timeout: nonnegative i32 seconds, otherwise rejected.

## What podup does not defend

podup is not a sandbox. The points below are documented limits an operator or auditor must know about.

- A compose file that asks for a privilege podup has no policy on. A `cap_add: [SYS_ADMIN]`, `pid: host`, `network_mode: host`, or `runtime: /path/to/binary` is forwarded to libpod as-is. podup emits a `tracing::warn!` per active host-binding / privilege-escalation mode (`network_mode: host`, `privileged: true`, `pid`/`ipc`/`uts`/`cgroup`/`userns_mode: host`, and the `container:<id>` namespace-sharing form). The `config` command surfaces the same modes at default log level. The actual gate is libpod's own validator.
- Compose-sourced paths are unconfined by design. `label_file`, `env_file`, `extends.file`, `include`, `secrets.file`, `build.context`, and the bind sources in `volumes:` accept `../` and absolute paths; the spec treats them as trusted operator input.
- Inline-secret `file:` sources have a point-in-time lifetime. The bytes are read at `up`, copied into a Podman-native secret, persist until replaced on a later `up` or removed on a `down`.
- Two `podup` invocations on Windows are not serialised. The per-project advisory lock is taken with `flock` on Unix; on Windows no OS lock is held.

## What the live integration lane validates

The `podman-lane` workflow boots Fedora qemu VMs in the runner with full systemd, once per supported Podman major (Fedora 44 for Podman 5, rawhide for Podman 6). Each VM runs the integration suite as a rootless user. The lane is a **required status check** on every pull request; runtime-irrelevant PRs skip VM boot. Per-major `.github/podman-known-failures-<major>` files classify the failures the lane still sees; any test that fails without appearing on its major's list is reported as an unexpected regression.

Unit/mocked tests cannot establish every libpod behavior; live tests and fuzzing add evidence, not exhaustive coverage.

## Reporting

Report vulnerabilities privately via the repository's **Security tab → Report a vulnerability** (never a public issue). See the organization [security policy](https://github.com/Glyndor/.github/blob/main/SECURITY.md) for response targets.