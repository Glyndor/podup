# Command reference

Every `podup` subcommand, its options, and what it does. Run `podup <command>
--help` for the same information at the terminal.

```
podup [GLOBAL OPTIONS] <COMMAND> [COMMAND OPTIONS] [SERVICE...]
```

## Global options

These apply to every command and may appear before the subcommand.

| Option | Env | Description |
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
`depends_on`).

| Option | Description |
|---|---|
| `-d, --detach` | Run containers in the background. |
| `--build` | Build images before starting. |
| `-w, --watch` | After starting, watch for changes per `develop.watch`. |
| `--remove-orphans` | Remove containers for services no longer in the file. |
| `--no-recreate` | Leave already-running containers in place. |
| `--force-recreate` | Recreate containers even if their config is unchanged. |
| `--no-deps` | Do not start the `depends_on` services of the named services. |
| `--scale <SERVICE=N>` | Override a service's replica count for this run. Repeatable. |

### `down`
Stop and remove containers. `-v, --volumes` also removes named volumes declared
in the compose file; `--remove-orphans` also removes containers for services no
longer in the file.

### `start` / `stop` / `restart`
Start existing stopped containers, stop running ones without removing them, or
restart them. `start`/`stop` accept a trailing service list; `restart [SERVICE]`
restarts everything or one service. `restart --no-deps` skips cascade-restarting
dependents that declare a `depends_on` restart condition.

### `scale <SERVICE=N>...`
Set the number of running containers for one or more services, creating missing
replicas and removing surplus ones. A service that publishes a **fixed host
port** cannot be scaled past one replica (only one container can bind a host
port) — the command fails fast and tells you to drop the host port (`- "80"`, so
Podman assigns one per replica), front it with a reverse proxy, or stay at one
replica.

### `create`
Create the containers for services without starting them (like `up` stopped at
the create step). `--build` builds images first; `--force-recreate` recreates
unchanged containers; `--no-recreate` leaves existing ones in place. Accepts a
trailing service list. A later `up`/`start` runs the created containers.

### `build`
Build or rebuild service images (optionally only the named services).

### `push`
Push each service's `image:` to its registry (services without an image are
skipped). Credentials come from `podman login`. `--ignore-push-failures`
continues after a failure; `--tls-verify false` allows an insecure/HTTP
registry (omit it to keep Podman's default verification). Accepts a trailing
service list.

### `rm`
Remove stopped service containers. `-f, --force` stops and removes running ones.

### `kill`
Send a signal to service containers. `-s, --signal <SIG>` sets the signal
(default `SIGKILL`).

### `pause` / `unpause`
Pause running service containers, or resume paused ones.

## Running commands

### `run <SERVICE> [COMMAND...]`
Run a one-off command in a new container for the service.

| Option | Description |
|---|---|
| `--rm` | Remove the container after it exits (default: true). |
| `-d, --detach` | Run in the background. |
| `-e, --env <KEY=VAL>` | Set an environment variable. Repeatable. |
| `--name <NAME>` | Override the container name. |
| `--service-ports` | Publish the service's declared ports (off by default). |

### `exec <SERVICE> <COMMAND...>`
Execute a command in a running service container.

### `cp <SRC> <DST>`
Copy files between a container and the host. Use `SERVICE:PATH` for the
container side, e.g. `podup cp web:/app/data ./local`.

## Inspection

| Command | Description |
|---|---|
| `ps` | List project containers. |
| `ls` | List podup compose projects on the host. `-a/--all` includes stopped projects, `-q/--quiet` prints names only, `--format table\|json`. Needs no compose file. |
| `top` | Show running processes of service containers. |
| `stats` | Live resource usage (CPU, memory, network, block I/O, PIDs) for service containers. `--no-stream` prints one snapshot; a trailing service list narrows it. |
| `port <SERVICE> <PRIVATE_PORT>` | Print the public binding for a port. `--proto` sets `tcp`/`udp` (default `tcp`). |
| `images` | List images used by services. |
| `logs [SERVICE]` | View container output. `-f/--follow` streams new output, `-n/--tail <N>` limits to the last N lines, `--since`/`--until` bound by time, `-t/--timestamps` prefixes each line. |
| `config` | Print the resolved compose file (after substitution, extends, include). |
| `pull` | Pull images for all services. |

## Generate

### `generate quadlet [-o <DIR>]`
Translate the compose file into Podman Quadlet unit files — one `.container` per
service plus `.network` and `.volume` units. Without `-o, --output` the units
print to stdout; warnings about fields with no Quadlet mapping go to stderr.

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

## Self-update

### `update`
Replace the running binary with the latest signed release.

| Option | Description |
|---|---|
| `--check` | Report whether a newer release exists; install nothing. |
| `--force` | Reinstall even if the latest release is not newer. |

Verification fails closed: a missing key, bad Ed25519 signature, or SHA-256
mismatch aborts before the installed binary is touched. See
[self-update.md](self-update.md) for the trust model.

## Shell completions

### `completions <SHELL>`
Print a shell completion script to stdout for `bash`, `zsh`, `fish`,
`powershell`, or `elvish`. The Debian package installs the bash/zsh/fish files
automatically; otherwise source the output from your shell startup:

```bash
podup completions bash > ~/.local/share/bash-completion/completions/podup
podup completions zsh  > "${fpath[1]}/_podup"
podup completions fish > ~/.config/fish/completions/podup.fish
```

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
| `1` | A command failed (parse error, Podman error). |
| `2` | `update` failed to verify or install a release. |
| other | `run` propagates the container's own exit code verbatim. |
