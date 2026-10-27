# Command reference, continued

[Lifecycle and container commands](commands.md).

## Images

### `pull [SERVICE...]`
Pull images for the named services, or all services if none are given.

| Flag | Description | Default |
|---|---|---|
| `-q, --quiet` | Suppress image-pull progress output. | off |
| `--ignore-pull-failures` | Continue pulling the remaining services after a failure. | off |
| `--include-deps` | Also pull images for the named services' `depends_on` services. | off |
| `--policy <POLICY>` | Pull policy, overriding per-service `pull_policy`: `always`, `missing`, `never`, `newer`, `build`. | per service |

Service `pull_policy` values `daily`, `weekly`, `every_<n>[wdhms]` and the podup extension `refresh` map to `newer`, including Quadlet `Pull=newer`. Podup checks the registry and keeps the local image when current, potentially using more network than the interval implies. It emits one warning per service: `pull_policy "<value>" is treated as "newer": podup does not track when an image was last pulled`.

Warnings name the effective policy after an engine override. Fixed overrides and registry autoupdate suppress the time-policy warning. Build services use or build their local image; Quadlet prebuilt services use `Pull=never` and emit no time-policy warning.

The podup extension `pull_refresh_after` is an optional string, validated and otherwise unused, with no extra warning for `refresh` and no warning under fixed policies. The accepted grammar is `([0-9]+[wdhms])+`, or `every_([0-9]+[wdhms])+` for a policy, using ASCII digits and lowercase units with a checked total of at most 4294967295 seconds. Empty numbers, malformed values, fractions, subsecond units, signs, whitespace and overflow are refused with the service, value and accepted values. `up`, `run`, `create`, `pull`, `build`, standalone `watch`, watch rebuilds and pod recreation validate before mutation; `config` and the Quadlet export gate validate too. `config --no-interpolate` also validates these two fields strictly and refuses unresolved variables. CLI policy options retain the fixed choices in the table above.


### `push [SERVICE...]`
Push each service's `image:` to its registry. Credentials come from Podman's auth file (`podman login`).

| Flag | Description | Default |
|---|---|---|
| `-q, --quiet` | Suppress the push progress output. | off |
| `--ignore-push-failures` | Continue after a failure. | off |
| `--tls-verify <BOOL>` | Verify the registry TLS cert; `false` allows an insecure/HTTP registry. | Podman default |

## Generate

### `generate quadlet`
Translate the compose file into Podman Quadlet unit files: one `.container` per service plus `.network` and `.volume` units. `gen` is an alias.

Exported build services use sibling `.build` units and require Podman 5.2+. Other units support Podman 5.0; later Quadlet settings use equivalent `PodmanArgs=` flags. Generation warns when the project needs a newer version.

The `.build` unit carries the compose `build:` fields. `context` and `dockerfile` become `SetWorkingDirectory=` and `File=`, `target` becomes `Target=`, `network` becomes `Network=`, `labels` become `Label=`, `args` become `PodmanArgs=--build-arg`, `pull` becomes `Pull=`, `platforms` become `Arch=` and `Variant=` (the section carries one pair, so only the first platform is exported), and `secrets` become `Secret=` (a top-level secret with a `file:` source, which is what `podman build --secret` reads). The fields with no `[Build]` key of their own travel as `PodmanArgs=` flags: `cache_from` and `cache_to` (with `--layers`, which buildah requires before it honours them), `no_cache`, `shm_size`, `extra_hosts`, `tags`, `additional_contexts`, `ssh`, `ulimits` and `isolation`. The remaining ones (`privileged`, `entitlements`, `provenance`, `sbom`) have no `podman build` equivalent and are reported, like every other field the export cannot carry.

A value the flag cannot take is reported instead of being passed through: an `additional_contexts` entry that names a service (`service:other`) instead of pointing at a path or a URL, a `ulimits` name outside the Linux rlimit set, an `isolation` that is not one of the buildah isolation types (`oci`, `rootless`, `chroot`), and a `secrets` entry whose id or path holds a comma (the `Secret=` value is a comma-separated option list).

In pod mode, shared-field warnings use `x-podman-pod: <field>: <reason>; the value stays on each container`. Reasons include service disagreement, empty or blank DNS entries, and static pod addresses without exactly one declared network. See the [pod field mapping](docker-migration.md#pod-mode-x-podman-pod-true).

| Flag | Description | Default |
|---|---|---|
| `-o, --output <DIR>` | Directory to write the unit files into. Omit to print to stdout. | stdout |

```bash
podup generate quadlet -o ~/.config/containers/systemd
```

Quadlet units are consumed by systemd, so they only run on Linux.

#### Jobs in `generate quadlet`

A job with at least one exportable `schedule` entry gets `<project>-<job>.container` and one `<project>-<job>-<n>.timer` per entry (`n` counts from 1, in the order of the list). The container is written by the code that writes a service's unit, so `command`, `environment`, volumes and limits come out the same. It is `Type=oneshot`, has no `Restart=` (a set restart policy is reported) and no `[Install]` section: the timer starts it. A job with only `manual` writes nothing and is reported once. A `depends_on` towards a service or towards a job that is exported becomes `After=` and `Requires=`/`Wants=`; towards a job without a unit it is dropped with a warning only when the entry has `required: false`. A required dependency (the default) on a job that has no unit blocks the dependent: it is not exported and each of its entries is reported, because the timer could run it without the dependency having run. The block repeats up a chain of dependencies. `autostart` installs services only.

Quadlet only turns its own unit types into services and ignores `.timer` files. Copy the timers to the systemd unit directory (`~/.config/systemd/user`), run `systemctl --user daemon-reload` and `systemctl --user enable --now <name>.timer`. The `.container` stays with the other Quadlet units.

| Schedule key | Becomes |
|---|---|
| `cron` | `OnCalendar=`, for the subset in [Jobs and schedules](docker-migration.md#jobs-and-schedules) |
| `timezone` | a suffix of `OnCalendar=` when it is `UTC`, `GMT` or an `Area/Location` name (letters, digits and `_ + - /`, at most 64 bytes, no `..`); absent means the host zone |
| `missed_fires` | `Persistent=true` for `one` (the default), `Persistent=false` for `skip` |
| `concurrency` | `forbid` is how a systemd timer behaves; `queue` has no timer form and the entry is not exported |

Every timer sets `AccuracySec=1s`, because systemd otherwise spreads a timer by up to a minute. A job whose `container_name` is also used by another generated unit is not exported: the timer starts it with `--replace` and would remove that container. Around daylight saving changes systemd does not behave like cron (a run set for the skipped spring hour does not happen that day), so each job exported with the host zone or a zone with summer time gets one warning; see [Jobs and schedules](docker-migration.md#jobs-and-schedules).

Removing a `schedule` entry and generating again does not delete the old `<project>-<job>-<n>.timer`. If it was installed it keeps firing. With `-o`, `generate quadlet` warns about each timer in the directory that carries this project's marker and is not generated now; disable it with `systemctl --user disable --now <name>.timer` and delete the file.

Each entry that is not exported gets a warning that names the job, the position and the reason, and no unit. With `x-podman-pod: true` no job is exported, because a job is not part of the pod.

## Watch

### `watch`
Watch for file changes and react per each service's `develop.watch` rules. (`up --watch` does the same after starting the stack.) Actions: `sync`, `rebuild`, `restart`, `sync+restart`, `sync+exec`.

Every rule that watches a changed path runs, so two services watching the same directory each get their own copy of the change. A rebuild or a restart runs once per service or container per batch of changes.

Patterns use `.dockerignore` syntax. Context ignore files match paths relative to the build context; rule include/ignore patterns match relative to the rule's path, with last match winning. Legacy project-relative patterns may apply when no rule pattern matches, with a warning; fallback never overrides a re-include.

Sync copies paths that exist and removes deleted ones, a deleted directory with the entries under it; watching starts before initial sync. Copies into a writable bind of the watched path are skipped with a warning, while more-specific mounts and restart/exec actions still apply. Symlinks remain links; file-to-directory replacements remain directories.

On queue overflow, watch resyncs `initial_sync` rules and warns about work it could not replay. Save those files again to retry.

## Maintenance

### `config`
Print the resolved compose file (after substitution, extends, include, and `env_file`). `convert` is an alias.

`config` folds `env_file` into `environment`. Later files win; `environment` overrides them. Bare keys remain valueless for host inheritance.

| Flag | Description | Default |
|---|---|---|
| `--format <FMT>` | `yaml` or `json`. | `yaml` |
| `--services` | List service names, one per line. | off |
| `--volumes` | List named volumes, one per line. | off |
| `--images` | List the images services use, one per line. | off |
| `--profiles` | List the profiles the file declares, one per line. | off |
| `--hash <SERVICES>` | Print a stable per-service config hash for the given comma-separated services, or `'*'` for all. | none |
| `--no-normalize` | Accepted for compatibility; `config` always emits the normalized form. | off |
| `-q, --quiet` | Only validate; print nothing. | off |
| `--no-interpolate` | Leave `${VAR}` placeholders literal; pull policy fields still require valid resolved values. | off |
| `--resolve-image-digests` | Rewrite each service `image:` to its registry digest. | off |

`config --resolve-image-digests` and `autostart uninstall --purge` also contact Podman.

### `audit`
Print one row per container listing every hardening gap the compose file leaves open. Read-only; never contacts Podman or changes runtime behavior.

Every service is one container, and so is every `pre_start` step: an init step runs its own container and is reported under `<service>/pre_start[i]`, so a step that asks for `privileged: true` or mounts a host path is a finding about that step. A file without `pre_start` has one row per service, as before.

Default output is a table. Services without findings show `-`; a project with no findings prints `no findings`. `--format json` emits a findings array (empty as `[]`). `--strict` exits 1 when findings are present, 0 otherwise.

```sh
podup audit --list-checks --format json | jq -r '.checks[].id'
```

`--list-checks` lists this build's checks without loading Compose. It conflicts with `--strict`/`--wildcard-binds`. `port_published_on_wildcard` is opt-in.

| Check id | Fires when | Notes |
|---|---|---|
| `privileged` | `privileged: true` | Grants extended host privileges; under rootless Podman reduced but never incidental. |
| `host_namespace` | `network_mode: host`, or `pid`/`ipc`/`uts`/`cgroup`/`userns_mode: host` | One finding per active mode. |
| `dangerous_capability` | `cap_add` carries a capability from the dangerous list | The list includes `SYS_ADMIN`, `ALL`, `SYS_MODULE`, `DAC_READ_SEARCH`, `SYS_RAWIO`, `SYS_PTRACE`, `NET_ADMIN`, `SYS_BOOT`, `MKNOD`, `SYSLOG`, `AUDIT_CONTROL`, `AUDIT_WRITE`, `SETFCAP`; names are matched case-insensitively with a `CAP_` prefix stripped. |
| `writable_root` | `read_only` is not `true` | Compose's default is writable. |
| `no_cap_drop_all` | `cap_drop` does not contain `ALL` | Without it the runtime's default capability set stays. |
| `no_new_privileges_off` | `security_opt` lacks `no-new-privileges:true` | Both spellings (`no-new-privileges:true` and `no-new-privileges` alone, the Podman form) are accepted. |
| `no_pids_limit` | `pids_limit` unset | A fork bomb can exhaust the host's process table. |
| `no_memory_limit` | The effective memory limit is missing, unparseable, or negative | A top-level `mem_limit` takes precedence over `deploy.resources.limits.memory`. |
| `no_userns` | `userns_mode` unset | Set `auto` explicitly for a private subordinate UID range; see the migration guide. |
| `secret_in_environment` | `environment:` key whose name contains a secret-bearing segment (`PASSWORD`, `SECRET`, `TOKEN`, `KEY`, case-insensitive) and a value is set | Every set value is flagged, including resolved `${VAR}` interpolations and empty strings. Bare inherited keys (no value) are not. A name ending in `FILE` is exempt only when its value starts with `/`, `./`, or `../`. |
| `port_published_on_all_interfaces` | A port is published without a host IP | The bind falls on every host interface. |
| `sensitive_bind_mount` | A bind mount exposes a sensitive host path: a container runtime socket, or `/proc`, `/sys`, `/dev`, `/etc`, `/boot`, `/root`, or a runtime directory holding a socket | |
| `unpinned_image` | `image:` with no tag, with tag `latest`, or `latest` without a digest | An `@sha256:` digest counts as pinning regardless of the tag. |
| `port_published_on_wildcard` | A port is published with an explicit wildcard host IP (`0.0.0.0` or `::`) | Off by default; enable with `--wildcard-binds`. |
| `no_restart_policy` | Neither `restart:` nor `deploy.restart_policy:` is set | An explicit `restart: "no"` is a deliberate choice and stays silent. |
| `no_init` | `init:` is not `true` | PID 1 is the app; orphans become zombies. |
| `no_health_action` | A non-disabled `healthcheck:` has no `x-podman-on-failure` extension | An unhealthy container stays unhealthy. |
| `swap_unbounded` | A memory limit is in effect but `memswap_limit` is absent, `-1`, or differs from the memory limit | The service can page to disk. |
| `no_cpu_limit` | Neither `cpus:`, `deploy.resources.limits.cpus:`, nor `cpu_quota:` gives a limit | One service can take every core of the host. |

| Flag | Description | Default |
|---|---|---|
| `--format <FMT>` | `table` or `json`. | `table` |
| `--strict` | Exit 1 when any finding is present, 0 otherwise. Conflicts with `--list-checks` at parse time. | off |
| `--list-checks` | Enumerate the checks this build carries and exit. Takes no compose file. | off |
| `--wildcard-binds` | Enable the `port_published_on_wildcard` check. Conflicts with `--list-checks` at parse time. | off |

### `completions <SHELL>`
Print a shell completion script to stdout for `bash`, `zsh`, `fish`, `powershell`, or `elvish`. The Debian package installs the bash/zsh/fish files automatically.

```bash
# bash: create the per-user completions directory on first use
mkdir -p ~/.local/share/bash-completion/completions
podup completions bash > ~/.local/share/bash-completion/completions/podup

# fish: fish looks under ~/.config/fish/completions by default
mkdir -p ~/.config/fish/completions
podup completions fish > ~/.config/fish/completions/podup.fish
```

Zsh example (note that this shell has a different convention):

```zsh
mkdir -p ~/.local/share/zsh/site-functions
podup completions zsh > ~/.local/share/zsh/site-functions/_podup
# add to .zshrc before compinit:
#   fpath=(~/.local/share/zsh/site-functions $fpath)
```

### `update`
Replace the running binary with the latest signed release. Available on direct-binary installs only: the subcommand is feature-gated and the Debian package is built without it, so `podup update` on an apt install is missing outright.

| Flag | Description | Default |
|---|---|---|
| `--check` | Report whether a newer release exists; install nothing. Queries GitHub release metadata but downloads no release assets. | off |
| `--force` | Reinstall even if the latest release is not newer. | off |

The signed manifest and selected digest are verified before replacement. The post-install version test attempts rollback on mismatch. See [self-update.md](self-update.md) for freshness limits and verification.

### `autostart` (alias `boot`)
Manage a boot-time autostart unit for this compose project: rootless, user-scope `systemctl --user` (enable lingering with `loginctl enable-linger`). The subcommand is `install`, `uninstall`, `status`, or `rebuild`; `--mode` and `--auto-update` belong to `install`. See the autostart guide for setup details.

| Subcommand | Description |
|---|---|
| `install` | Install (and, by default, enable + start) the autostart unit(s) for this project. Writes only under `${XDG_CONFIG_HOME:-~/.config}`. |
| `uninstall` | Remove whichever mode is installed (auto-detected). Legacy Quadlets without ownership markers are skipped. `--purge` also tears the stack down and drops its volumes. |
| `status` | Report this project's unit and session state. |
| `rebuild [service]` | Quadlet mode only: rebuild the built image(s) and restart the container(s). Omit the argument to rebuild every built service. |

| Flag (`install`) | Description | Default |
|---|---|---|
| `--mode <MODE>` | Backend: `service`, `quadlet` or `start`; see the autostart guide for prerequisites. | `service` |
| `--no-start` | Service/start: do not enable or start. Quadlet: still build and write boot wiring, but do not start containers. | off |
| `--auto-update <FREQ>` | Service-only timer running `podup up -d` hourly, daily or weekly. | none |
| `--dry-run` | Print what would be written and run; change nothing. | off |

### `version`
Print version information. `podup --version` prints the same.

| Flag | Description | Default |
|---|---|---|
| `--short` | Print only the version number. | off |
| `--format <FMT>` | `pretty` or `json`. | `pretty` |

### Aliases

`remove` (`rm`), `volume` (`volumes`), `image` (`images`), `log` (`logs`), `resume` (`unpause`), `convert` (`config`), `gen` (`generate`, so `podup gen quadlet`) and `boot` (`autostart`) are accepted as aliases of the canonical names above.

### Override discovery

When neither `-f/--file` nor `COMPOSE_FILE` is set, podup also loads the first override file it finds next to the compose file: `compose.override.yaml`, `compose.override.yml`, `docker-compose.override.yaml` or `docker-compose.override.yml`.

## Progress output

Progress is on stderr. A stderr terminal gets an animated board; other sinks get append-only lines. `--ansi`/`NO_COLOR` affect color, not animation. Reused resources report `Exists`; replacements report `Recreating`/`Recreated`. No-op transitions are suppressed; unfinished work is retained at command end. Empty operations say why no action occurred.

## Diagnostics

Warnings/errors go to stderr with a `podup:` prefix. Unsupported-field warnings are on by default; `RUST_LOG=debug` enables tracing. Internal errors request a bug report with secrets redacted. Invalid `umask` values name the service and advise quoting an octal string. Values containing `$` also report that the variable did not resolve. `config --no-interpolate` refuses an unresolved `umask` placeholder. `build.no_cache_filter` produces one warning per service: podup cannot filter the build cache per stage through the libpod build API, so the setting is ignored.

## Jobs

The top-level `jobs:` map declares containers that run to completion. A job takes every key a service takes, plus a required `triggers:` block. A `jobs:` that is empty or null declares nothing, the same as no key.

| Trigger | Meaning |
|---|---|
| `manual` | Boolean, `true` by default (the text `true` and `false` is read too, since `config` prints a boolean that way). `manual: false` makes `podup run` refuse the job, also when another job being run depends on it. |
| `schedule` | List of crontab strings, or objects with `cron` (required), `timezone`, `concurrency` (`forbid` by default, or `queue`) and `missed_fires` (`one` by default, or `skip`). |

`triggers` must declare at least one trigger once all `-f` files are merged, so an override file may leave it out. A missing or empty `triggers`, a trigger of the wrong type, an unknown key, a cron expression that is blank, longer than 256 bytes or has a character outside `0-9 A-Z a-z * / , - ? # @` and space, and an invalid `concurrency` or `missed_fires` are errors that name the job.

podup validates a schedule and does not fire it. `run`, `up` and `config` print one warning per job that has a schedule, whatever its number of entries, and `--no-warn` does not silence it. `generate quadlet` exports the schedules it can as systemd timers (see [Jobs in `generate quadlet`](#jobs-in-generate-quadlet)); the others run when you start the job with `podup run`, or from a scheduler you provide.

- `podup up` starts services only and never starts a job.
- `podup run NAME` looks for a service first and then a job. A job runs as a one-off container, with the flags of `run`, after the services and jobs it lists in `depends_on`. `run` waits for the `depends_on` conditions of the job before creating its container. A dependency that is a job with `manual: false` is an error naming both jobs.
- A job may depend on services and on other jobs. A service cannot depend on a job.
- Services and jobs share one namespace: the same name in both is an error.
- `extends` of a job points at another job, and `extends` of a service points at another service.
- `profiles` work as they do for services: the job is ignored unless one of its profiles is active. A job never activates a service: one that depends on a service behind an inactive profile is ignored with a warning, in `config`, `audit`, `generate quadlet` and `up` alike, and `podup run` starts that service with the job. `config` prints the jobs that remain, with their triggers, and `audit` checks them with the same checks as services.
- `down` removes the containers a job left (a `run` of it, or one started for a job that depends on it), and `down --rmi` removes the images of jobs.
- Variable interpolation, `include`, several `-f` files and `!override`/`!reset` treat a job like a service. Across `-f` files a later non-empty `schedule` replaces the earlier list; `schedule: !reset []` empties it, `schedule: !override [...]` and `triggers: !override {...}` replace the list or the whole block, and `triggers: !reset` clears it. A job left with no trigger is an error.

## Environment

### Compose and connection

| Variable | Description |
|---|---|
| `COMPOSE_FILE` | Path-separator-delimited list of compose files (`--file`). |
| `COMPOSE_PROJECT_NAME` | Default project name (`--project`). |
| `COMPOSE_PROFILES` | Default active profiles (`--profile`). |
| `PODMAN_SOCKET` | Podman socket path (`--socket`). |
| `DOCKER_HOST` | Docker-compatible fallback for the Podman socket, used only when `PODMAN_SOCKET` is unset. Must be a local `unix://` socket (or `npipe://` on Windows). |
| `PODUP_LIBPOD_POOL` | HTTP/1.1 connection-pool size for the libpod client (`--connection-pool-size`). Connection-pool size; see `--connection-pool-size`. `PODUP_LIBCOD_POOL` is read as a fallback when the new name is unset. |
| `PODUP_MAX_REPLICAS` | Per-service replica cap (default 256). A positive integer overrides it; zero or invalid values use the default. |

### Runtime, colour, config

| Variable | Description |
|---|---|
| `RUST_LOG` | Log verbosity filter. Unset: warnings and errors, except `watch` which uses `info`. `RUST_LOG=podup=off` disables podup tracing. |
| `NO_COLOR` | Disables colour output under `--ansi auto`. `--ansi always` overrides it. |
| `TERM`, `COLORTERM` | Used to detect a colour-capable terminal; selects the wide palette on truecolor/256color terminals and Windows. |
| `XDG_RUNTIME_DIR` | Linux socket discovery tries `XDG_RUNTIME_DIR`, then `/run/user/<uid>`. Unix locks use `XDG_RUNTIME_DIR/podup` when the absolute runtime directory passes ownership/permission checks; unset or relative values use `temp_dir()/podup-<euid>`, and unsafe absolute values abort. Windows uses `%TEMP%/podup` without OS locking. |
| `XDG_CONFIG_HOME` | Base for the `~/.config` path used by `autostart install` when writing user units. Defaults to `$HOME/.config` when unset. |
| `HOME` | Resolves the macOS `podman machine` socket candidates and the default `XDG_CONFIG_HOME`. |
| `USER`, `LOGNAME` | The login user for `loginctl` session and linger queries. |
| `PATH` | Searched for `podman` when `autostart install --mode start` writes its unit. |
| `SCOOP` | On Windows, the Scoop root the `podup update` package-manager detection consults first. |

## Podman extensions

| Key | Where | What it does | Portable |
|---|---|---|---|
| `x-podman-on-failure` | under a service's `healthcheck:` | `none`, `kill`, `restart` or `stop`. Default `none`. | yes |
| `x-podman-pod` | top level | `true` puts every service of the project into one Podman pod named after the project. | yes |
| `x-podman-autoupdate` | under a service | `registry` or `local`; see [Auto-update](#auto-update). | yes |
| `noexec`, `nosuid`, `nodev` | under a long-form volume's `volume:` | mount-hardening flags; see [Per-mount hardening options](docker-migration.md#per-mount-hardening-options-noexec-nosuid-nodev). | no |

### Auto-update

`x-podman-autoupdate` sets `io.containers.autoupdate`; Quadlet exports `AutoUpdate`. Registry pulls and local image changes follow the table below. Autostart scheduling is described in the [autostart guide](autostart.md#auto-update).

| Value | What it does |
|---|---|
| `registry` | The container carries `io.containers.autoupdate=registry`, and `podup up` pulls the image with policy `newer` so a moved tag recreates the container. For a service without `build:`, it does so even when the image is already on disk; a service with `build:` builds its image instead of pulling it. `--pull <policy>` wins over the extension. |
| `local` | The container carries `io.containers.autoupdate=local`: Podman's auto-update compares the container's image with the local image of the same name and restarts the unit when they differ. |

### Pods

`x-podman-pod: true` at the top level puts every service of the project into one Podman pod, named after the project, with a shared network namespace. The pod uses the project networks, or pasta/slirp4netns when every service declares the same such `network_mode`.

The infra container stays running when the last service exits, in both API and Quadlet deployments. What changes inside the pod:

- Services reach each other on `localhost`. `up` adds one `<service>:127.0.0.1` host entry per service. They share one port space, so two services cannot listen on the same container port.
- `ports:` are published by the pod.
- Only the network namespace is shared. UTS and IPC stay per container.
- `up` records a hash of the pod's ports, networks and host entries. When it changes, `up` recreates the pod and every member container, which discards their writable layers.

What is refused, before anything is created:

- `network_mode` on any service;
- a service whose `networks:` set differs from another service's;
- two services publishing the same host port;
- services that disagree on `userns_mode`.

### Healthcheck timing on a `service_healthy` gate

When `up` waits on `depends_on: {condition: service_healthy}`, podup runs healthchecks itself so hosts without systemd timers can progress. It runs at the parsed interval (minimum 100ms, otherwise 2s), reads status every 150ms, and waits `interval × retries + start_period`. `--wait-timeout` does not extend this dependency budget.

For an unhealthy container that should recover, use `x-podman-on-failure: restart`. The reported Podman kill/unless-stopped combination leaves it exited; service-mode autostart does not supervise container exits. Invalid values error in `up`/`create`; Quadlet generation warns and omits them.

## Accepted for compatibility

These flags parse and are validated, so a script written against docker compose runs unchanged, but podup does not act on them.

| Flag | Why it does nothing |
|---|---|
| `build --progress <STYLE>` | podup renders build output one way. The value is still validated, so a typo is rejected rather than silently ignored. |
| `config --no-normalize` | `config` always emits the normalized form. |
| `cp -a, --archive` | Ownership/permission preservation is not meaningful for a rootless copy. |
| `attach --no-stdin`, `--sig-proxy`, `--detach-keys` | `attach` streams output only; stdin is never attached. Use `exec`/`run` for an interactive session. |

## Exit status

| Code | Meaning |
|---|---|
| `0` | Success. |
| `1` | A command failed (Podman error, runtime failure). |
| `2` | Command-line usage error (unknown flag, bad argument). |
| `3` | `update` failed to verify or install a release. |
| `126` | `run`/`exec`: the command exists but is not executable. |
| `127` | `run`/`exec`: the command was not found. |
| `130` | An attached `up` was ended by SIGINT or SIGTERM. |
| other | `run` and attached `exec` propagate the command's own exit code verbatim; `wait` returns the last non-zero code it saw. `up --abort-on-container-exit` does the same with the first container to exit; `up --exit-code-from SERVICE` does the same with the named service. |

Attached `up` returns 1 if logs end while a container still runs. Abort/exit-code-from stops other containers and leaves them available for later up/down. SIGINT and SIGTERM both return 130 after stopping the project.

`stats --format json` differs from docker on purpose. JSON uses numeric CPU fields and separate I/O counters; Docker-formatted string consumers need adapting.

A streaming command that loses its connection fails. `logs`, `stats`, an attached `up`, `run`, and `events` exit `1` if output ends while the container is still running, or status cannot be checked. `run` reports a transport failure when no exit result is available. Clean container completion and logs reader closure succeed. `events` returns 0 only at the clean end of a past window with both `--since`/`--until`; other feed endings or transport failures return 1.

`watch` warns and continues after action failures; it exits 0 unless startup fails. Its exit status does not certify every action succeeded.