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
| `--env-file <PATH>` | | Env file(s) for interpolation. External extends chains retain the root compose file's resolved env paths. Repeatable; later files win. Replaces `.env` rather than adding to it. Process environment takes precedence. |
| `--no-warn` | | Suppress host-binding / privilege-escalation warnings during `up`/`create`/`run`/`exec`. The port-exposure warning is shown only by `up`/`create`/`run`/`scale`/`watch`/`generate quadlet`/`autostart` unless `--no-warn` is set, and by `config` regardless of `--no-warn`. `config` still surfaces the active modes at the default log level. |
| `-h, --help` | | Per-command help. The `help [COMMAND]` subcommand renders the same text. |
| `-V, --version` | | Print `podup version vX.Y.Z`. Same as `podup version`. |

**Profiles activate their dependencies.** A service left out by profile filtering still starts when a running service declares `depends_on` on it, transitively. docker compose rejects that file instead. See [docker-migration.md](docker-migration.md#a-depends_on-target-behind-an-inactive-profile).

**Identity colours.** Service names keep consistent colours across commands: 20 on truecolor/256color terminals or Windows, otherwise six ANSI colours. `--ansi` and `NO_COLOR` control colour output. Status colours distinguish created/healthy, removed/failed and stopped resources.

## Defaults

| Compose key | Default | Notes |
|---|---|---|
| `logging` | `driver: k8s-file` + `max-size: 10m` | Use journald to delegate rotation; `max-size: -1` removes this cap but host `log_size_max` may still apply. Positive `max-size` without driver selects k8s-file; nonpositive/absent size keeps the host driver. Podman keeps no rotated history. Quadlet uses the same default. |

## Lifecycle

`pre_start` runs one ephemeral init container per step, in list order, before the service container is created by `up`, `run` or `create`, and before a `watch` rebuild recreates it. Unset fields inherit from the service; collections are completed and explicit scalars replace inherited values. Each step must exit zero before the next step or the service proceeds. Failure names the service and step number (for example, `service 'web': pre_start step 1: exited with code 7`), blocks dependents and removes the init container. Init containers are removed after success too, regardless of `run --no-rm`. Restart policies and service lifecycle lists do not repeat init work. Steps inherit neither ports/expose nor static IP/MAC addresses; explicit step values are kept. Steps without their own image or build reuse the service image. A step that declares its own image does not inherit the service's build, and a step that declares its own build does not inherit the service's image: the step is pulled or built on its own. The one-off settings of a `run` (its command, `--entrypoint`, terminal, ad-hoc mounts and ad-hoc labels) describe the throwaway container and never reach the init steps; the user, working directory and environment do. A failed init leaves the previous service running. An already-running service is left alone; `up` runs init steps before starting an existing stopped service.

`create` runs init steps to completion but leaves the service container stopped. Quadlet export warns that `pre_start` is not exported: `ExecStartPre=` cannot represent a separate container per step.

`podup audit` walks the init steps too: each step is a container of its own and is reported under `<service>/pre_start[i]`, so a step that asks for `privileged: true` or mounts a host path is a finding about that step.

### `up`
Create and start all services (or only the named ones, plus their transitive `depends_on`). Accepts a trailing service list. Jobs are never started by `up`.

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
Restart reuses the existing containers and does not apply compose changes (ports, environment, image); use `up -d` for that.

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

### `events [SERVICE...]`
Stream Podman events for this project's containers. The default output is one `TIME TYPE ACTION NAME` line per event; the timestamp is the engine event's time in the local zone (UTC marked `Z` if no zone is available; missing times are blank). `--format json` emits one object per line (NDJSON) with an added `Action` field (the original `status` stays), normalized image-remove/death verbs, and `exitCode` copied only for `died` events. `--json` is a hidden deprecated alias for `--format json`. With one or more service names the feed is narrowed client-side to events for those services; an unknown name fails with `service 'NAME' not found` before the stream is opened.

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
Run a one-off command in a new container for the service, or for a job of the same name (see [Jobs](commands-reference.md#jobs)).

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

<a id="pull-service"></a><a id="push-service"></a>[Open this section](commands-reference.md#images).

## Generate

<a id="generate-quadlet"></a>[Open this section](commands-reference.md#generate).

## Watch

<a id="watch"></a>[Open this section](commands-reference.md#watch).

## Maintenance

<a id="config"></a><a id="audit"></a><a id="completions-shell"></a><a id="update"></a><a id="autostart-alias-boot"></a><a id="version"></a><a id="aliases"></a><a id="override-discovery"></a>[Open this section](commands-reference.md#maintenance).

## Progress output

[Open this section](commands-reference.md#progress-output).

## Diagnostics

[Open this section](commands-reference.md#diagnostics).

## Environment

<a id="compose-and-connection"></a><a id="runtime-colour-config"></a>[Open this section](commands-reference.md#environment).

## Podman extensions

<a id="auto-update"></a><a id="pods"></a><a id="healthcheck-timing-on-a-service_healthy-gate"></a>[Open this section](commands-reference.md#podman-extensions).

## Accepted for compatibility

[Open this section](commands-reference.md#accepted-for-compatibility).

## Exit status

[Open this section](commands-reference.md#exit-status).
