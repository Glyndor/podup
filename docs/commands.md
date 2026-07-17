# Command reference

This page lists every `podup` command, its options, and what it does. Run
`podup <command> --help` for the same information at the terminal. The
[global options](#global-options) below apply to every command.

```
podup [GLOBAL OPTIONS] <COMMAND> [COMMAND OPTIONS] [SERVICE...]
```

## Global options

These appear before the subcommand and may also come from the environment.

| Flag | Env | Description |
|---|---|---|
| `-f, --file <PATH>` | `COMPOSE_FILE` | Compose file. Repeatable; later files merge over earlier ones. When unset, the compose-spec precedence list is probed: `compose.yaml`, `compose.yml`, `docker-compose.yaml`, `docker-compose.yml`. |
| `-p, --project <NAME>` | `COMPOSE_PROJECT_NAME` | Project name, prefixing container/network/volume names. When unset: the top-level `name:`, then the sanitized project-directory basename. |
| `--socket <PATH>` | `PODMAN_SOCKET` | Podman socket path; overrides auto-detection. |
| `--profile <NAMES>` | `COMPOSE_PROFILES` | Active profiles, comma-separated. |
| `--project-directory <PATH>` | | Base directory for relative paths (env_file, build context, bind mounts, config/secret sources). Defaults to the compose file's directory. |
| `--env-file <PATH>` | | Extra env file for interpolation. Repeatable; later files win. The process environment and a project `.env` still take precedence. |

## Lifecycle

### `up`
Create and start all services (or only the named ones, plus their transitive
`depends_on`). Accepts a trailing service list.

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
| `--pull <POLICY>` | Pull policy before starting: `always`, `missing`, `never`. | per service |
| `--no-build` | Do not build images, even for services with a `build:` section. | off |
| `--quiet-pull` | Suppress image-pull progress output. | off |
| `--wait` | Wait until services are running/healthy before returning. | off |
| `--no-start` | Create the containers but do not start them. | off |
| `--timestamps` | Prefix attached log lines with a timestamp (ignored with `-d`). | off |
| `-V, --renew-anon-volumes` | Recreate anonymous volumes instead of keeping the previous ones. | off |

```bash
podup up -d --build
```

### `down`
Stop and remove containers, networks, and (with `-v`) volumes.

| Flag | Description | Default |
|---|---|---|
| `-v, --volumes` | Also remove named volumes declared in the compose file. | off |
| `--remove-orphans` | Remove containers for services no longer in the file. | off |
| `--rmi <SCOPE>` | Also remove service images: `all`, or `local` (only those built from a `build:` section). | keep images |
| `-t, --timeout <SECS>` | Seconds to wait for containers to stop before killing them. | Podman default |

### `create`
Create the containers for services without starting them (like `up` stopped at
the create step). Accepts a trailing service list. A later `up`/`start` runs the
created containers.

| Flag | Description | Default |
|---|---|---|
| `--build` | Build images before creating containers. | off |
| `--force-recreate` | Recreate containers even if their config is unchanged. | off |
| `--no-recreate` | Leave existing containers in place. | off |

### `start`
Start existing stopped containers. Accepts a trailing service list.

| Flag | Description | Default |
|---|---|---|
| `--wait` | Wait until services are running/healthy before returning. | off |
| `--wait-timeout <SECS>` | Maximum seconds to wait with `--wait` before giving up. | no limit |

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
| `-q, --quiet` | Suppress the build output. | off |

## Inspection

### `ps`
List project containers.

| Flag | Description | Default |
|---|---|---|
| `-a, --all` | Include stopped containers. | running only |
| `-q, --quiet` | Print container IDs only. | off |
| `--format <FMT>` | `table` or `json`. | `table` |

### `ls`
List podup compose projects on the host. Needs no compose file.

| Flag | Description | Default |
|---|---|---|
| `-a, --all` | Include stopped projects. | running only |
| `-q, --quiet` | Print project names only. | off |
| `--format <FMT>` | `table` or `json`. | `table` |

### `logs [SERVICE...]`
View container output for the named services (or all).

| Flag | Description | Default |
|---|---|---|
| `-f, --follow` | Stream new output. | off |
| `-n, --tail <N>` | Show the last N lines. | all |
| `--since <TIME>` | Show logs since a timestamp or relative time (e.g. `10m`). | start |
| `--until <TIME>` | Show logs before a timestamp or relative time. | end |
| `-t, --timestamps` | Prefix each line with an RFC3339 timestamp. | off |

### `events`
Stream Podman events for this project's containers.

| Flag | Description | Default |
|---|---|---|
| `--format <FMT>` | `table` (a `TYPE ACTION NAME` summary) or `json` (one object per line). | `table` |

`--json` is a hidden deprecated alias for `--format json`.

### `top [SERVICE...]`
Show the running processes of service containers.

### `stats [SERVICE...]`
Live resource usage (CPU, memory, network, block I/O, PIDs) for service
containers.

| Flag | Description | Default |
|---|---|---|
| `--no-stream` | Print one snapshot and exit. | stream |

### `port <SERVICE> <PRIVATE_PORT>`
Print the public binding for a port.

| Flag | Description | Default |
|---|---|---|
| `--proto <PROTO>` | `tcp` or `udp`. | `tcp` |
| `--index <N>` | Target this replica (1-based) of a scaled service. | 1 |

### `images`
List images used by services.

| Flag | Description | Default |
|---|---|---|
| `-q, --quiet` | Print image IDs only. | off |
| `--format <FMT>` | `table` or `json`. | `table` |

### `volumes [SERVICE...]`
List the project's named volumes (a trailing service list narrows it to volumes
those services mount).

| Flag | Description | Default |
|---|---|---|
| `-q, --quiet` | Print volume names only. | off |
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
| `-u, --user <NAME\|UID[:GID]>` | Run the command as this user. | image default |
| `-w, --workdir <PATH>` | Working directory inside the container. | image default |
| `--entrypoint <CMD>` | Override the image entrypoint. | image default |
| `-v, --volume <SPEC>` | Bind-mount an extra volume (`HOST:CONTAINER[:OPTS]` or `NAME:CONTAINER`). Repeatable. | none |
| `-p, --publish <SPEC>` | Publish an extra port (`HOST:CONTAINER[/PROTO]`). Repeatable. | none |
| `-i, --interactive` | Keep STDIN open (accepted for compatibility; `run` still streams logs). | off |
| `-T, --no-TTY` | Disable pseudo-TTY allocation (accepted for compatibility; podup never allocates one). | off |
| `--no-deps` | Do not start the `depends_on` services before running. | off |

```bash
podup run --rm web sh -c 'echo hello'
```

### `exec <SERVICE> <COMMAND...>`
Execute a command in a running service container.

| Flag | Description | Default |
|---|---|---|
| `-e, --env <KEY=VAL>` | Set an environment variable. Repeatable. | none |
| `-u, --user <NAME\|UID[:GID]>` | Run the command as this user. | container default |
| `-w, --workdir <PATH>` | Working directory inside the container. | container default |
| `--privileged` | Give extended privileges to the command. | off |
| `-d, --detach` | Run the command in the background. | off |
| `-T, --no-TTY` | Disable pseudo-TTY allocation (accepted for compatibility; podup never allocates one). | off |
| `--index <N>` | Target this replica (1-based) of a scaled service. | 1 |

```bash
podup exec -u root web sh
```

### `cp <SRC> <DST>`
Copy files between a container and the host. Use `SERVICE:PATH` for the
container side, e.g. `podup cp web:/app/data ./local`.

| Flag | Description | Default |
|---|---|---|
| `--index <N>` | Target this replica (1-based) of a scaled service. | 1 |
| `-L, --follow-link` | Follow symlinks in the host source before copying into the container. | off |
| `-a, --archive` | Accepted for compatibility (no effect under rootless Podman). | off |

### `attach <SERVICE>`
Attach to a service container's output (stdout/stderr), streaming it until the
container exits or you detach.

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
Pause running service containers, or resume paused ones. `resume` is an alias
for `unpause`.

### `wait [SERVICE...]`
Block until the named service containers (default: all) stop, printing each
container's exit code as it does.

### `scale <SERVICE=N>...`
Set the number of running containers for one or more services, creating missing
replicas and removing surplus ones. A service that publishes a **fixed host
port** cannot be scaled past one replica — the command fails fast and tells you
to drop the host port (`- "80"`, so Podman assigns one per replica), front it
with a reverse proxy, or stay at one replica.

### `commit <SERVICE> <IMAGE>`
Commit a service container's current state to a new image reference
(`repo[:tag]`).

| Flag | Description | Default |
|---|---|---|
| `--index <N>` | Select a replica (1-based) of a scaled service. | 1 |

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
Push each service's `image:` to its registry (services without an image are
skipped). Credentials come from `podman login`.

| Flag | Description | Default |
|---|---|---|
| `--ignore-push-failures` | Continue after a failure. | off |
| `--tls-verify <BOOL>` | Verify the registry TLS cert; `false` allows an insecure/HTTP registry. | Podman default |

## Generate

### `generate quadlet`
Translate the compose file into Podman Quadlet unit files — one `.container` per
service plus `.network` and `.volume` units. `gen` is an alias for `generate`.

| Flag | Description | Default |
|---|---|---|
| `-o, --output <DIR>` | Directory to write the unit files into. Omit to print to stdout. | stdout |

```bash
podup generate quadlet -o ~/.config/containers/systemd
```

Quadlet units are consumed by systemd, so they only run on Linux. Generating
them on macOS or Windows is allowed (e.g. to deploy to a remote Linux host) but
prints a `podup: warning:` to stderr noting the files will not run on the host.

## Watch

### `watch`
Watch for file changes and react as configured by each service's
`develop.watch` rules. (`up --watch` does the same after starting the stack.)
The `action` of each rule may be:

| Action | Effect on change |
|---|---|
| `sync` | Copy the changed files into the running container. |
| `rebuild` | Rebuild the image and recreate the container. |
| `restart` | Restart the container without rebuilding. |
| `sync+restart` | Sync the files, then restart the container. |
| `sync+exec` | Sync the files, then run the rule's `exec` command in the container. |

## Maintenance

### `config`
Print the resolved compose file (after substitution, extends, include).
`convert` is an alias.

| Flag | Description | Default |
|---|---|---|
| `--format <FMT>` | `yaml` or `json`. | `yaml` |
| `--services` | List service names, one per line. | off |
| `-q, --quiet` | Only validate; print nothing. | off |
| `--no-interpolate` | Leave `${VAR}` placeholders literal. | off |
| `--resolve-image-digests` | Rewrite each service `image:` to its registry digest (`repo@sha256:...`). | off |

### `completions <SHELL>`
Print a shell completion script to stdout for `bash`, `zsh`, `fish`,
`powershell`, or `elvish`. The Debian package installs the bash/zsh/fish files
automatically; otherwise source the output from your shell startup:

```bash
podup completions bash > ~/.local/share/bash-completion/completions/podup
podup completions zsh  > "${fpath[1]}/_podup"
podup completions fish > ~/.config/fish/completions/podup.fish
```

### `update`
Replace the running binary with the latest signed release.

| Flag | Description | Default |
|---|---|---|
| `--check` | Report whether a newer release exists; install nothing. | off |
| `--force` | Reinstall even if the latest release is not newer. | off |

Verification fails closed: a missing key, bad Ed25519 signature, or SHA-256
mismatch aborts before the installed binary is touched. See
[self-update.md](self-update.md) for the trust model.

### `autostart` (alias `boot`)
Manage a boot-time autostart unit for this compose project — rootless,
user-scope `systemctl --user` (enable lingering with
`loginctl enable-linger` so the unit starts without a login session). See
[Rootless autostart](autostart.md) for the full setup, the two backends, and
running it under an isolated service account.

| Subcommand | Description |
|---|---|
| `install` | Install (and, by default, start) the autostart unit(s) for this project. Writes only under `${XDG_CONFIG_HOME:-~/.config}`. |
| `uninstall` | Remove whichever mode is installed (auto-detected). `--purge` also tears the stack down and drops its volumes. |
| `status` | Report this project's unit and session state. |
| `rebuild [service]` | Quadlet mode only: rebuild the built image(s) and restart the container(s). Omit the argument to rebuild every built service. |

| Flag (`install`) | Description | Default |
|---|---|---|
| `--mode <MODE>` | Autostart backend: `service` (one `Type=oneshot` unit running `podup up -d` at boot, `podup stop` on shutdown) or `quadlet` (one native Podman Quadlet unit per service, owned by systemd directly). | `service` |
| `--no-start` | Install the unit(s) but do not start them. | off |
| `--dry-run` | Print what would be written and run; change nothing. | off |

### `version`
Print version information, like `docker compose version`. `podup --version`
prints the same.

| Flag | Description | Default |
|---|---|---|
| `--short` | Print only the version number. | off |
| `--format <FMT>` | `pretty` or `json`. | `pretty` |

## Diagnostics

podup writes warnings and errors to **stderr**, prefixed with `podup:` (so the
emitter is identifiable in journald and multi-tool logs) while stdout stays a
clean pipe (e.g. the YAML from `config`, the units from `generate quadlet`).
Forward-compatibility warnings about unknown or unsupported compose fields are
shown by default; set `RUST_LOG=debug` for verbose tracing. An unexpected
internal error prints a `podup: internal error:` notice with a bug-report link
and a reminder to redact secrets before sharing logs.

## Environment

Every environment variable `podup` reads, in one place. Each compose variable
has an equivalent flag (see [Global options](#global-options)); the flag wins
when both are set.

| Variable | Description |
|---|---|
| `COMPOSE_FILE` | Path-separator-delimited list of compose files (`--file`). |
| `COMPOSE_PROJECT_NAME` | Default project name (`--project`). |
| `COMPOSE_PROFILES` | Default active profiles (`--profile`). |
| `PODMAN_SOCKET` | Podman socket path (`--socket`). |
| `DOCKER_HOST` | Docker-compatible fallback for the Podman socket, used only when `PODMAN_SOCKET` is unset. Must be a local `unix://` socket (or `npipe://` on Windows); a remote `tcp://`/`ssh://` value is rejected. |
| `RUST_LOG` | Log verbosity filter. Unset shows warnings and errors; e.g. `RUST_LOG=podup=info` or `RUST_LOG=podup=debug` for more detail. |

## Exit status

| Code | Meaning |
|---|---|
| `0` | Success. |
| `1` | A command failed (Podman error, runtime failure). |
| `2` | Command-line usage error (unknown flag, bad argument). |
| `3` | `update` failed to verify or install a release. |
| `126` | `run`/`exec`: the command exists but is not executable. |
| `127` | `run`/`exec`: the command was not found. |
| other | `run` propagates the container's own exit code verbatim. |
