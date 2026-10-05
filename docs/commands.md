# Command reference

Commands, options and environment settings. Run `podup <command> --help` for terminal help.

```
podup [GLOBAL OPTIONS] <COMMAND> [COMMAND OPTIONS] [SERVICE...]
```

`-f/--file` and `-p/--project` come before the command. `--socket`, `--connection-pool-size`, `--profile`, `--project-directory`, `--env-file`, `--ansi` and `--no-warn` work either side of it.

## Global options

| Flag | Env | Description |
|---|---|---|
| `-f, --file <PATH>` | `COMPOSE_FILE` | Compose file. Repeatable; later files merge over earlier ones. When unset, the compose-spec precedence list is probed: `compose.yaml`, `compose.yml`, `docker-compose.yaml`, `docker-compose.yml`. |
| `-p, --project <NAME>` (alias `--project-name`) | `COMPOSE_PROJECT_NAME` | Project name, prefixing container/network/volume names. When unset: the top-level `name:`, then the sanitized project-directory basename. |
| `--socket <PATH>` | `PODMAN_SOCKET` | Podman socket path; overrides auto-detection. |
| `--connection-pool-size <N>` | `PODUP_LIBPOD_POOL` | HTTP/1.1 connections the libpod client keeps open to the Podman socket. Streaming calls each take a dedicated connection outside this cap. Default 0 disables pooling. `PODUP_LIBCOD_POOL` is read as a fallback when the new name is unset. |
| `--profile <NAMES>` | `COMPOSE_PROFILES` | Active profiles, comma-separated. |
| `--project-directory <PATH>` | | Base directory for relative paths (env_file, build context, bind mounts, config/secret sources). Defaults to the compose file's directory. |
| `--ansi <WHEN>` | | Colour output: `auto`, `always` or `never`. `--ansi always` forces colour even into a pipe or file and overrides `NO_COLOR`; `--ansi auto` honours `NO_COLOR` and TTY detection. |
| `--env-file <PATH>` | | Env file(s) for interpolation. Repeatable; later files win. Replaces `.env` rather than adding to it. Process environment takes precedence. |
| `--no-warn` | | Suppress host-binding / privilege-escalation warnings during `up`/`create`/`run`/`exec`. `config` still surfaces the active modes at the default log level. |
| `-h, --help` | | Per-command help. The `help [COMMAND]` subcommand renders the same text. |
| `-V, --version` | | Print `podup version vX.Y.Z`. Same as `podup version`. |

**Profiles activate their dependencies.** A service left out by profile filtering still starts when a running service declares `depends_on` on it, transitively. docker compose rejects that file instead. See [docker-migration.md](docker-migration.md#a-depends_on-target-behind-an-inactive-profile).

**Identity colours.** Service names keep consistent colours across commands: 20 on truecolor/256color terminals or Windows, otherwise six ANSI colours. `--ansi` and `NO_COLOR` control colour output. Status colours distinguish created/healthy, removed/failed and stopped resources.

## Defaults

| Compose key | Default | Notes |
|---|---|---|
| `logging` | `driver: k8s-file` + `max-size: 10m` | Use journald to delegate rotation; `max-size: -1` removes this cap but host `log_size_max` may still apply. Positive `max-size` without driver selects k8s-file; nonpositive/absent size keeps the host driver. Podman keeps no rotated history. Quadlet uses the same default. |

## Lifecycle

### `up`
Create and start all services (or only the named ones, plus their transitive `depends_on`). Accepts a trailing service list.

A container that already exists is left in place when two facts both hold: its recorded config hash equals what the compose file renders now, and the image it is bound to is still the image its service resolves to. Either changing replaces it. `--no-recreate` keeps an existing container regardless; `--force-recreate` replaces it regardless.

| Flag | Description | Default |
|---|---|---|
| `-d, --detach` | Run containers in the background. | off |
| `--build` | Build images before starting. | off |
| `-w, --watch` | After starting, watch for changes per `develop.watch`. | off |
| `--remove-orphans` | Remove containers for services no longer in the file. | off |
| `--no-recreate` | Leave already-running containers in place. | off |
| `--force-recreate` | Recreate containers even if their config is unchanged. | off |
| `--no-deps` | Do not start the `depends_on` services of the named services. | off |
| `-t, --timeout <SECS>` | Seconds to wait for a container to stop when recreating. | Podman default |
| `--scale <SERVICE=N>` | Override a service's replica count for this run. Repeatable. | from file |
| `--pull <POLICY>` | Pull policy before starting: `always`, `missing`, `never`, `newer`, `build`. (`newer` is Podman's extension.) | per service |
| `--no-build` | Do not build images, even for services with a `build:` section. | off |
| `--quiet-pull` | Suppress image-pull progress output. | off |
| `--wait` | Wait until services are running/healthy before returning. | off |
| `--wait-timeout <SECS>` | Maximum seconds to wait with `--wait` before giving up. | no additional overall cap |
| `--no-start` | Create the containers but do not start them. | off |
| `--timestamps` | Prefix attached log lines with a timestamp (ignored with `-d`). | off |
| `-V, --renew-anon-volumes` | Recreate anonymous volumes instead of keeping the previous ones. | off |
| `--abort-on-container-exit` | Stop every container as soon as any of them exits; the process exit status is that container's exit code. Cannot be combined with `-d`, `--wait`, or `--watch`. | off |
| `--exit-code-from SERVICE` | Return the named service's exit code as podup's own. Implies `--abort-on-container-exit`; a service that does not exist in the compose file is rejected before any container is created. | off |

`--wait` implies `-d` and checks only each service's first replica. Health polling retains its per-service budget (interval × retries + start_period); `--wait-timeout` does not extend this dependency budget.

With `missing`, an observed local image skips redundant pulls. `always`/`newer` and platform-pinned services still pull; build services use or build their local image.

```bash
podup up -d --build
```

### `down`
Stop and remove containers, networks, and (with `-v`) volumes. With `-v`, podup removes the project's named volumes and the data in them; volumes declared `external: true` are left alone (measured on 2026-09-19).

| Flag | Description | Default |
|---|---|---|
| `-v, --volumes` | Also remove named volumes declared in the compose file. | off |
| `--remove-orphans` | Remove containers for services no longer in the file. | off |
| `--rmi <SCOPE>` | Also remove service images: `all`, or `local` (only those built from a `build:` section). | keep images |
| `-t, --timeout <SECS>` | Seconds to wait for containers to stop before killing them. | Podman default |

### `create`
Create the containers for services without starting them. Accepts a trailing service list.

| Flag | Description | Default |
|---|---|---|
| `--build` | Build images before creating containers. | off |
| `--force-recreate` | Recreate containers even if their config is unchanged. | off |
| `--no-recreate` | Leave existing containers in place. | off |
| `--no-deps` | Do not create the `depends_on` services of the named ones. | off |
| `--pull <POLICY>` | Pull policy before creating: `always`, `missing`, `never`, `newer`, `build`. | per service; otherwise `missing` |

### `start`
Start existing stopped containers. Accepts a trailing service list. `start --wait` extends the readiness wait by the timeout given; a `depends_on: {condition: service_healthy}` wait inside an `up` is not.

| Flag | Description | Default |
|---|---|---|
| `--wait` | Wait until services are running/healthy before returning. | off |
| `--wait-timeout <SECS>` | Maximum seconds to wait with `--wait` before giving up. | no additional overall cap |

### `stop`
Stop running containers without removing them. Accepts a trailing service list.

| Flag | Description | Default |
|---|---|---|
| `-t, --timeout <SECS>` | Seconds to wait for containers to stop before killing them. | Podman default |

### `restart`
Restart service containers (default: all, or the named ones).

| Flag | Description | Default |
|---|---|---|
| `-t, --timeout <SECS>` | Seconds to wait for containers to stop before killing them. | Podman default |
| `--no-deps` | Do not cascade-restart dependents that declare a `depends_on` restart condition. | off |

### `build`
Build or rebuild service images (optionally only the named services).

| Flag | Description | Default |
|---|---|---|
| `--no-cache` | Do not use the build cache. | off |
| `--pull` | Always attempt to pull a newer base image. | off |
| `--build-arg <KEY=VAL>` | Set a build-time variable. Repeatable. | none |
| `--progress <STYLE>` | `auto`, `plain` or `tty`. Validated but inert; see [accepted for compatibility](#accepted-for-compatibility). | `auto` |
| `--push` | Push each built image to its registry after a successful build. | off |
| `-q, --quiet` | Suppress the build output. | off |

Build progress goes to stderr. A terminal board shows each image's step and recent output; failure prints the full stream. `up --build` shares that board. Other stderr sinks receive stream lines prefixed by image tag. Image IDs go to non-terminal stdout only.

In the reported no-user-systemd WSL case, failed builds may need `cgroup_manager = "cgroupfs"`; see the migration guide's Windows section. On failure podup adds this hint if the API reports systemd as the cgroup manager.

## Inspection

### `ps`
List project containers.

| Flag | Description | Default |
|---|---|---|
| `-a, --all` | Include stopped containers. | running only |
| `-q, --quiet` | Print container IDs only. | off |
| `--format <FMT>` | `table` or `json`. | `table` |
| `--status <STATE>` | Show only containers in this state. Repeatable; folded together with any `status=` from `--filter`. | all |
| `--filter <KEY=VAL>` | `name=<NAME>` or `status=<STATE>`. An unknown key is an error. | none |
| `--services` | Print service names only. | off |
| `-s, --size` | Add a SIZE column with each container's on-disk footprint. | off |

An empty result prints `no containers` on stderr and leaves stdout empty in table output. Under `--format json` an empty result is `[]`; under `-q/--quiet` neither the note nor a row is printed.

### `ls`
List podup compose projects on the host. Needs no compose file.

| Flag | Description | Default |
|---|---|---|
| `-a, --all` | Include stopped projects. | running only |
| `-q, --quiet` | Print project names only. | off |
| `--format <FMT>` | `table` or `json`. | `table` |
| `--filter <FILTER>` | Keep only projects matching a predicate: `name=<NAME>` or `status=<running\|exited>`. Repeatable. | none |

An empty result prints `no projects` on stderr and leaves stdout empty in table output. Under `--format json` an empty result is `[]`; under `-q/--quiet` neither the note nor a row is printed.

### `logs [SERVICE...]`
View container output for the named services (or all).

| Flag | Description | Default |
|---|---|---|
| `-f, --follow` | Stream new output. | off |
| `-n, --tail <N>` | Show the last N lines. `all` opts back into the full stream. | 100 |
| `--since <TIME>` | Show logs since a timestamp or relative time (e.g. `10m`). | start |
| `--until <TIME>` | Show logs before a timestamp or relative time. | end |
| `-t, --timestamps` | Prefix each line with an RFC3339 timestamp. | off |
| `--no-color` | Monochrome prefix even on a colour-capable stdout. | off |
| `--no-log-prefix` | Drop the `{service} \| ` tag entirely. | off |

### `events`
Stream Podman events for this project's containers. The default output is one `TIME TYPE ACTION NAME` line per event; the timestamp is the engine event's time in the local zone (UTC marked `Z` if no zone is available; missing times are blank). `--format json` emits one object per line (NDJSON) with an added `Action` field (the original `status` stays), normalized image-remove/death verbs, and `exitCode` copied only for `died` events. `--json` is a hidden deprecated alias for `--format json`.

| Flag | Description | Default |
|---|---|---|
| `--format <FMT>` | `table` (a plain `TIME TYPE ACTION NAME` summary) or `json`. | `table` |
| `--filter <FILTER>` | Keep only events matching a predicate (`KEY=VALUE`). Repeatable. Forwarded to libpod; an unknown key is rejected by the engine. | none |
| `--since <TIME>` | Only stream events at or after this timestamp or relative time. | stream start |
| `--until <TIME>` | End of the window. Only closes the feed when paired with `--since` and already elapsed. | no end |

**Bounding a feed needs both flags.** A past window such as `--since 2h --until -1h` ends the feed (measured on Podman 5.7.0 on 2026-09-23 with `--since 30s --until -1s`); `--until` alone, `--since` alone, and any `--until` in the future leave it following indefinitely. podup warns when `--until` is given without `--since`. This also decides the exit code; see [Exit status](#exit-status).

### `top [SERVICE...]`
Show the running processes of service containers.

| Flag | Description | Default |
|---|---|---|
| `--format <FMT>` | `table` or `json` (an array of `{Container, Titles, Processes}`). | `table` |

### `stats [SERVICE...]`
Live resource usage (CPU, memory, network, block I/O, PIDs) for service containers. On a terminal the table repaints in place; anywhere else each frame is appended. `--format json` never repaints.

| Flag | Description | Default |
|---|---|---|
| `--no-stream` | Print one snapshot and exit. | stream |
| `-a, --all` | Include non-running containers as zeroed rows. | running only |
| `--no-trunc` | Do not truncate long container names. | truncate at 32 |
| `--format <FMT>` | `table` or `json`. While streaming, `json` is NDJSON. | `table` |

### `port <SERVICE> <PRIVATE_PORT>`
Print the public binding for a port.

| Flag | Description | Default |
|---|---|---|
| `--proto <PROTO>` (alias `--protocol`) | `tcp` or `udp`. | `tcp` |
| `--index <N>` | Target this replica (1-based) of a scaled service. | 1 |

### `images`
List images used by services.

| Flag | Description | Default |
|---|---|---|
| `-q, --quiet` | Print image IDs only. | off |
| `--format <FMT>` | `table` or `json`. | `table` |

An empty result (no services with `image:` or `build:`) prints `no images` on stderr and leaves stdout empty in table output. Under `--format json` an empty result is `[]`; under `-q/--quiet` neither the note nor a row is printed.

### `volumes [SERVICE...]`
List the project's named volumes. `EXTERNAL` is highlighted when it reads `yes`: podup neither creates nor deletes an external volume, so those are the ones a `down -v` leaves standing.

| Flag | Description | Default |
|---|---|---|
| `-q, --quiet` | Print volume names only. | off |
| `-s, --size` | Add SIZE and RECLAIMABLE columns. | off |
| `--format <FMT>` | `table` or `json`. | `table` |

## Container operations

### `run <SERVICE> [COMMAND...]`
Run a one-off command in a new container for the service.

| Flag | Description | Default |
|---|---|---|
| `--rm` | Remove the container after it exits. | on |
| `--no-rm` | Keep the one-off container after it exits. | off |
| `-d, --detach` | Run in the background. | off |
| `-e, --env <KEY=VAL>` | Set an environment variable. Repeatable. | none |
| `--name <NAME>` | Override the container name. | generated |
| `-P, --service-ports` | Publish the service's declared ports. | off |
| `-l, --label <KEY=VAL>` | Add a label to the one-off container. Repeatable. | none |
| `-u, --user <NAME\|UID[:GID]>` | Run the command as this user. | service configuration, then image default |
| `-w, --workdir <PATH>` | Working directory inside the container. | service configuration, then image default |
| `--entrypoint <CMD>` | Override the image entrypoint. | service configuration, then image default |
| `-v, --volume <SPEC>` | Bind-mount an extra volume. Repeatable. | none |
| `-p, --publish <SPEC>` | Publish an extra port. Repeatable. | none |
| `-i, --interactive` | Keep the container's STDIN open (`stdin_open`). Whether a live terminal is attached is decided by stdin/stdout being terminals and by `-T`, not by this flag. | off |
| `-T, --no-TTY` (alias `--no-tty`) | Disable pseudo-TTY allocation. | off |
| `--no-deps` | Do not start the `depends_on` services before running. | off |

```bash
podup run --rm web sh -c 'echo hello'
```

> **Differs from docker on purpose.** `run` removes the container by default here; `docker compose run` keeps it unless `--rm` is passed.

Attached `run` removes its container by default; use `--no-rm` to keep it. Detached `run` keeps it. TTY allocation requires both stdin and stdout to be terminals; `-T` or `-d` disables it. A PTY merges stdout/stderr and uses CRLF, so redirects and pipes use ordinary streams.

### `exec <SERVICE> <COMMAND...>`
Execute a command in a running service container.

| Flag | Description | Default |
|---|---|---|
| `-e, --env <KEY=VAL>` | Set an environment variable. Repeatable. | none |
| `-u, --user <NAME\|UID[:GID]>` | Run the command as this user. | container default |
| `-w, --workdir <PATH>` | Working directory inside the container. | container default |
| `--privileged` | Give extended privileges to the command. | off |
| `-d, --detach` | Run the command in the background. | off |
| `-T, --no-tty` (alias `--no-TTY`) | Disable pseudo-TTY allocation. | off |
| `--index <N>` | Target this replica (1-based) of a scaled service. | 1 |

`podup exec db psql` is interactive when stdin and stdout are terminals. `-T` disables TTY allocation; there are no `-i`/`-t` flags. Pipes and redirects use ordinary streams.

```bash
podup exec -u root web sh
```

### `cp <SRC> <DST>`
Copy files between a container and the host. Use `SERVICE:PATH` for the container side.

| Flag | Description | Default |
|---|---|---|
| `--index <N>` | Target this replica (1-based) of a scaled service. | 1 |
| `-L, --follow-link` | Follow symlinks in the host source before copying into the container. | off |
| `-a, --archive` | Accepted for compatibility (no effect under rootless Podman). | off |

### `attach <SERVICE>`
Attach to a service container's output (stdout/stderr), streaming it until the container exits or you detach. Output only; stdin is never attached.

| Flag | Description | Default |
|---|---|---|
| `--index <N>` | Target this replica (1-based) of a scaled service. | 1 |
| `--no-stdin` | Accepted for compatibility; stdin is never attached anyway. | off |
| `--sig-proxy [<BOOL>]` | Accepted for compatibility; no effect. | off |
| `--detach-keys <KEYS>` | Accepted for compatibility; no effect. | none |

### `kill [SERVICE...]`
Send a signal to service containers.

| Flag | Description | Default |
|---|---|---|
| `-s, --signal <SIG>` | Signal to send. | `SIGKILL` |
| `--remove-orphans` | Then remove containers for services no longer in the file. | off |

### `rm [SERVICE...]`
Remove stopped service containers.

| Flag | Description | Default |
|---|---|---|
| `-f, --force` | Remove even running containers (stop first). | off |
| `-v, --volumes` | Also remove anonymous volumes attached to them. | off |
| `-s, --stop` | Stop the containers (gracefully) before removing them. | off |

### `pause [SERVICE...]` / `unpause [SERVICE...]`
Pause running service containers, or resume paused ones. `resume` is an alias for `unpause`.

### `wait [SERVICE...]`
Block until the named service containers (default: all) stop, printing one line per container as it exits.

| Flag | Description | Default |
|---|---|---|
| `--format <FORMAT>` | `table` for the aligned columns, or `json` for NDJSON. | table |

A project with nothing to wait on prints nothing and exits 0.

### `scale <SERVICE=N>...`
Set the number of running containers for one or more services, creating missing replicas and removing surplus ones. A service that publishes a **fixed host port** cannot be scaled past one replica; the command fails fast and tells you to drop the host port (`- "80"`, so Podman assigns one per replica), front it with a reverse proxy, or stay at one replica.

### `commit <SERVICE> <IMAGE>`
Commit a service container's current state to a new image reference.

| Flag | Description | Default |
|---|---|---|
| `--index <N>` | Select a replica (1-based) of a scaled service. | 1 |
| `-m, --message <MSG>` | Commit message recorded on the image. | none |
| `-a, --author <AUTHOR>` | Author recorded on the image. | none |
| `-c, --change <INSTRUCTION>` | Apply a Dockerfile instruction to the created image. Repeatable. | none |
| `-p, --pause [<BOOL>]` | Pause the container during commit for a consistent snapshot. `--pause=false` snapshots it live. | **on** |

### `export <SERVICE>`
Export a service container's filesystem as a tar archive.

| Flag | Description | Default |
|---|---|---|
| `-o, --output <FILE>` | Write to a file instead of stdout. | stdout |
| `--index <N>` | Select a replica (1-based) of a scaled service. | 1 |

## Images

### `pull [SERVICE...]`
Pull images for the named services, or all services if none are given.

| Flag | Description | Default |
|---|---|---|
| `-q, --quiet` | Suppress image-pull progress output. | off |
| `--ignore-pull-failures` | Continue pulling the remaining services after a failure. | off |
| `--include-deps` | Also pull images for the named services' `depends_on` services. | off |
| `--policy <POLICY>` | Pull policy, overriding per-service `pull_policy`: `always`, `missing`, `never`, `newer`, `build`. | per service |

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

Generated build services use sibling `.build` units and require Podman 5.2+. Other units support Podman 5.0; later Quadlet settings use equivalent `PodmanArgs=` flags. Generation warns when the project needs a newer version.

| Flag | Description | Default |
|---|---|---|
| `-o, --output <DIR>` | Directory to write the unit files into. Omit to print to stdout. | stdout |

```bash
podup generate quadlet -o ~/.config/containers/systemd
```

Quadlet units are consumed by systemd, so they only run on Linux.

## Watch

### `watch`
Watch for file changes and react per each service's `develop.watch` rules. (`up --watch` does the same after starting the stack.) Actions: `sync`, `rebuild`, `restart`, `sync+restart`, `sync+exec`.

Patterns use `.dockerignore` syntax. Context ignore files match paths relative to the build context; rule include/ignore patterns match relative to the rule's path, with last match winning. Legacy project-relative patterns may apply when no rule pattern matches, with a warning; fallback never overrides a re-include.

Sync copies paths that exist and removes deleted ones; watching starts before initial sync. Copies into a writable bind of the watched path are skipped with a warning, while more-specific mounts and restart/exec actions still apply. Symlinks remain links; file-to-directory replacements remain directories.

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
| `--no-interpolate` | Leave `${VAR}` placeholders literal. | off |
| `--resolve-image-digests` | Rewrite each service `image:` to its registry digest. | off |

`config --resolve-image-digests` and `autostart uninstall --purge` also contact Podman.

### `audit`
Print one row per service listing every hardening gap the compose file leaves open. Read-only; never contacts Podman or changes runtime behavior.

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

Warnings/errors go to stderr with a `podup:` prefix. Unsupported-field warnings are on by default; `RUST_LOG=debug` enables tracing. Internal errors request a bug report with secrets redacted.

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