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

### `down`
Stop and remove containers. `-v, --volumes` also removes named volumes declared
in the compose file.

### `start` / `stop` / `restart`
Start existing stopped containers, stop running ones without removing them, or
restart them. `start`/`stop` accept a trailing service list; `restart [SERVICE]`
restarts everything or one service.

### `build`
Build or rebuild service images (optionally only the named services).

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
| `top` | Show running processes of service containers. |
| `port <SERVICE> <PRIVATE_PORT>` | Print the public binding for a port. `--proto` sets `tcp`/`udp` (default `tcp`). |
| `images` | List images used by services. |
| `logs [SERVICE]` | View container output. `-f, --follow` streams new output. |
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

## Watch

### `watch`
Watch for file changes and sync, rebuild or restart services as configured by
each service's `develop.watch` rules. (`up --watch` does the same after
starting the stack.)

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
