# Migrating from Docker Compose to podup

Run existing Compose projects on Podman with `podup up`. The sections below cover supported fields, rootless differences and ignored settings.

```sh
cd my-project          # the directory holding docker-compose.yml
podup up
```

## Works out of the box

The following Compose fields are translated onto Podman's libpod API. Fields outside this list still parse (an unknown key never errors, per the Compose spec's forward-compatibility rule) and podup reports any it cannot honor during `up`; see the accepted-but-has-no-effect section.

| Category | Keys |
|---|---|
| Images / build | `image`, `build` (context, dockerfile, args, target, cache_from, cache_to, secrets, labels, network), `pull_policy` (time policies map to `newer`), `pull_refresh_after` (strict syntax only) |
| Execution | `command`, `entrypoint`, `working_dir`, `user`, `umask`, `platform`, `tty`, `stdin_open` |
| Environment | `environment`, `env_file` (dotenv format), variable substitution (`${VAR:-default}`) |
| Networking | `ports`, `expose`, `networks`, `network_mode`, `hostname`, `domainname`, `dns`, `dns_opt`, `dns_search`, `extra_hosts` |
| Volumes | `volumes` (bind, named, tmpfs, npipe), `tmpfs`, `volumes_from` |
| Secrets / configs | `secrets`, `configs` (file, inline content, environment source, and `external: true` Podman-native secrets) |
| Dependencies | `depends_on` (with `condition:` set to `service_started`, `service_healthy` or `service_completed_successfully`) |
| Health checks | `healthcheck` (test, interval, timeout, retries, start_period, disable) |
| Lifecycle hooks | `post_start`, `pre_stop` |
| Restart policies | `restart`, `deploy.restart_policy` (`condition`, `max_attempts`) |
| Replicas / scale | `deploy.replicas`, `scale:` |
| Resource limits | `deploy.resources`, `mem_limit`, `cpus`, `cpu_shares`, `cpu_quota`, `pids_limit`, `ulimits`, `blkio_config` |
| Devices | `devices`, `device_cgroup_rules` |
| GPU | `deploy.resources.reservations.devices`, `gpus:`; see the note below |
| Security | `cap_add`, `cap_drop`, `security_opt`, `read_only`, `privileged`, `userns_mode` |
| Namespaces | `pid`, `ipc`, `uts`, `cgroup`, `shm_size` |
| Metadata | `labels`, `annotations`, `container_name`, `profiles` |
| Logging | `logging` (driver + options) |
| Compose features | `extends`, `include`, YAML anchors, x-extensions, `develop.watch` |

`umask` sets the initial process permission mask. Use a string of one to four octal digits, such as `"0022"`, `"022"` or `"22"` (range `0000` to `7777`). Only integer `0` is accepted; quote every other numeric value to avoid YAML radix ambiguity. Validation follows normal interpolation and refuses leftover variables with the service name and a quote hint before container or pod replacement. Normal interpolation can turn numeric text into a YAML integer, so use a leading-zero value such as `0022`. With `config --no-interpolate`, a placeholder in `umask` is refused because the typed field still requires a valid mask. Valid output is padded to four digits. Omitted or null leaves the runtime default; explicit zero sends `"0000"` and masks no permission bits. The quadlet export uses `PodmanArgs=--umask=<value>`, compatible with Podman 5.0. Scalar inheritance, `!reset` and `!override` apply. Explicit `umask: null` changes the configuration hash once on upgrade because the previously unknown null key is now omitted. Tagged payloads use normal interpolation before merge tags are stripped; unresolved masks still fail validation.

### Pod mode (`x-podman-pod: true`)

Pod mode writes one `.pod` unit for the project. Fields that every service declares with the same value move from each `.container` unit to that shared pod unit:

| Compose field | Pod unit value |
|---|---|
| `hostname` | `--hostname` |
| `dns` | `--dns` |
| `dns_opt` | `--dns-option` |
| `dns_search` | `--dns-search` |
| `extra_hosts` | `--add-host` |
| `userns_mode` | `--userns` |
| `shm_size` | `--shm-size` |
| `networks.*.ipv4_address` | `--ip` |
| `networks.*.ipv6_address` | `--ip6` |
| `networks.*.aliases` | `--network-alias` |

Service labels stay on every container and never become pod labels. All volumes stay on their containers, including named, bind, anonymous and long-form tmpfs mounts. The `tmpfs` field also stays container-level. Mounting a named volume on the infra container could change which image initializes its contents.

If services disagree, or only some services declare the field, podup emits a warning and keeps the value on each container. It never chooses one service's value for the whole pod. DNS lists move only when every entry is nonempty and nonblank after control characters are stripped; otherwise they remain on the containers with a warning, preserving native list-reset behavior. Static IPv4 and IPv6 addresses move only when the pod attaches to exactly one declared network. With multiple networks, including unused top-level declarations, addresses remain on the containers with a warning. The native Quadlet keys added after 5.0 travel through `PodmanArgs=`, so the unit still loads on Podman 5.0.

Promoted native container values, including hostname and DNS, discard boundary whitespace after control characters are stripped. Extra hosts and network aliases keep the same quoted argument values as their container emitters, including boundary spaces.

The pod keeps `PodmanArgs=--exit-policy=continue`, so its infra container stays running when the last service exits. Pod-level stop timeout has no Compose source and stays unset. A service's `stop_grace_period` remains a container-level timeout.

### Time-based pull policies

`pull_policy:` accepts `always`, `never`, `missing` (alias `if_not_present`), `build`, Podman's `newer`, `daily`, `weekly`, `every_<n>[wdhms]`, and the podup extension `refresh`.

`daily`, `weekly`, `every_<n>[wdhms]` and `refresh` map to libpod policy `newer`, including Quadlet `Pull=newer`. The registry is checked for a newer image and the local image is used when current. This may use more network than the declared interval. Podman reports image build time, not last-pull time, so podup does not compare image ages. Each service using a time policy emits one warning:

```text
pull_policy "<value>" is treated as "newer": podup does not track when an image was last pulled
```

Warnings name the effective policy after an engine override. Fixed overrides and registry autoupdate suppress the time-policy warning. Build services use or build their local image; Quadlet prebuilt services use `Pull=never` and emit no time-policy warning.

The podup extension `pull_refresh_after` is an optional string, validated for syntax and otherwise unused with every policy. Setting it with `refresh` adds no further effect or warning; with a fixed policy it produces no warning. Its grammar is `([0-9]+[wdhms])+`, and the `every_` policy grammar is `every_([0-9]+[wdhms])+`: ASCII digits, lowercase units, one or more pairs, at most 4294967295 total seconds. Zero, repeated units and any unit order are accepted. Empty counts, unknown units, fractions, subsecond units, signs, whitespace and overflow are refused with the service, original value and accepted values. Validation runs before mutation for `up`, `create`, `run`, `pull`, `build`, standalone `watch`, watch rebuilds and pod recreation, and also covers `config` and the Quadlet export gate. `config --no-interpolate` also validates these two fields strictly: unresolved variables are refused instead of rendered. This limitation avoids accepting malformed values in ordinary `config`.

```yaml
services:
  web:
    image: nginx:1.27
    pull_policy: refresh
    pull_refresh_after: 24h
```

`!reset`, `!override null` and plain null clear an inherited `pull_refresh_after` through `extends` and multiple `-f` files. Services omitting the field retain their serialized configuration. An explicit `pull_refresh_after: null` may change the configuration hash once on upgrade.

### GPU reservations are host-dependent

`deploy.resources.reservations.devices` and the `gpus:` shorthand are honored, but only for **NVIDIA** GPUs, and only when the host exposes them through CDI (the NVIDIA Container Toolkit must be installed and the CDI spec generated). Reservations for other drivers or capabilities are warned about and skipped.

### Interpolation across compose files

`!override` and `!reset` payloads are interpolated before their merge directives are applied, including nested mappings and lists. External `extends.file` chains receive the referring compose file's interpolation mode and resolved explicit env paths, without rebasing those paths at each external directory. The root file anchors env paths to its compose directory. `config --no-interpolate` preserves placeholders in external bases too, subject to typed-field validation such as `umask` and pull policies. Included files retain their original interpolation context, including local env files, when resolving external bases. Includes are merged with parent definitions winning before same-file inheritance is resolved.

### External secrets and configs

A secret or config declared `external: true` is mounted from an existing Podman secret rather than from a value in the project tree, the recommended pattern for production credentials. Create the secret before running:

```sh
: "${DB_PASSWORD:?Set a nonempty database password}"
printf '%s' "$DB_PASSWORD" | podman secret create db_password -
```

```yaml
services:
  db:
    image: postgres:16
    environment:
      POSTGRES_PASSWORD_FILE: /run/secrets/db_password
    secrets:
      - db_password
secrets:
  db_password:
    external: true
```

Secrets appear under `/run/secrets/<name>`; configs default to `/<name>`, and a long-form `target:` overrides either. If the named Podman secret does not exist, `podup up` fails fast rather than starting a container without it. Use a top-level `name:` when the Podman secret is named differently from the compose reference.

## Behaves differently under rootless Podman

These are Podman behaviours, not podup limitations; they apply equally to any Podman-based tool. Each one is something to expect, with the workaround.

### `network_mode: bridge`

| | |
|---|---|
| **What to expect** | docker-compose attaches the container to Docker's predefined shared `bridge` network. Podman reads `--network bridge` as "create a fresh, isolated bridge netns", so the container has outbound connectivity but **cannot reach its project siblings** by name or IP. |
| **Workaround** | Remove `network_mode: bridge` and let the container join the project's default network, or declare a shared `networks:` entry the services share. podup emits a warning when it sees `network_mode: bridge`. |

Before: siblings unreachable under Podman because each container sits in its own isolated network namespace.

```yaml
services:
  web:
    image: nginx:alpine
    network_mode: bridge
  api:
    image: alpine:3.22
    command: ["sleep", "infinity"]
    network_mode: bridge
```

After: the two services share an `app` network and resolve each other by name.

```yaml
networks:
  app: {}
services:
  web:
    image: nginx:alpine
    networks: [app]
  api:
    image: alpine:3.22
    command: ["sleep", "infinity"]
    networks: [app]
```

### Published ports and the client address

On a bridge network (the project network, `bridge`, or `x-podman-pod`), `rootlessport` forwards published ports, so the container sees every client as one internal address. `network_mode: pasta` keeps the client's address; options go after a colon, as in `network_mode: "pasta:-T,15432"`. A pod runs on pasta when every service declares the same `network_mode: pasta` (options allowed) and then keeps the client's address. podup warns on `up` and `config` when a service publishes ports through such a proxy.

### Privileged ports (< 1024)

Rootless containers cannot bind host ports below 1024 unless the kernel allows it. Use a higher host port, or lower the kernel floor. sysctl changes are temporary unless configured under `/etc/sysctl.d/`.

```sh
sudo sysctl net.ipv4.ip_unprivileged_port_start=80
```

```yaml
services:
  web:
    image: nginx:alpine
    ports:
      - "8080:80"
```

### UID/GID mapping

By default, root inside the container maps to your host user; a process runs as the image's or service's `user`. Files written as root in the container appear owned by your user on the host. Bind-mount permissions reflect your host user's access.

### `userns_mode: auto` gives each container its own UID range

With its default configuration, rootless Podman maps container root onto your host user. Leaving `userns_mode` unset uses that default mapping, so containers share the same host identity for UID 0. Podman's `--userns=auto` allocates a private range of subordinate UIDs and GIDs. Podup requests that allocation when you explicitly set the compose key:

```yaml
services:
  web:
    image: registry.example.com/web:1.4.2
    userns_mode: auto
```

`auto` needs available subuid/subgid ranges; request `auto:size=65536` only if it fits. Existing `keep-id`/`nomap` allocations may exhaust them. `keep-id` creates are serialized per command after observed Podman 5.7.0 mapping failures (2026-09-27); separate commands may overlap. Mount permissions must allow mapped IDs. `:U` recursively changes host ownership; separate ranges need compatible permissions for shared volumes.

### Volume SELinux labels

On SELinux-enforcing systems, bind mounts require relabeling. Append `:z` (shared) or `:Z` (private) to the volume spec:

```yaml
services:
  app:
    image: docker.io/library/alpine:3.20
    volumes:
      - ./data:/app/data:Z
```

### Per-mount hardening options (`noexec`, `nosuid`, `nodev`)

The short form carries these as raw mount options (`cache:/app/cache:noexec`), and the long form accepts them as booleans under `volume:`:

```yaml
volumes:
  cache: {}
services:
  web:
    image: nginx:alpine
    volumes:
      - type: volume
        source: cache
        target: /app/cache
        volume:
          noexec: true
          nosuid: true
          nodev: true
```

The long-form spelling is a podup extension (the Compose Specification defines no per-mount hardening flags there), so a compose file using it is not portable back to `docker compose`, which rejects the unknown keys. `generate quadlet` carries the options into the exported unit.

### `network_mode: host`

Attaches the container to your user's network namespace, not a privileged host namespace. Traffic is still limited to your user's capabilities.

### `network_mode: none`

Supported. The container gets a loopback interface only.

### `mac_address:` at the service level

The Compose spec deprecated the top-level `mac_address` field in favour of per-network configuration. podup still honours it (for backward compatibility) and applies it to the primary network, but logs a deprecation warning. Move it under `networks:` to silence the warning:

Before:

```yaml
services:
  web:
    image: nginx:alpine
    mac_address: "02:42:ac:11:00:02"
```

After:

```yaml
networks:
  default: {}
services:
  web:
    image: nginx:alpine
    networks:
      default:
        mac_address: "02:42:ac:11:00:02"
```

## Accepted, and podup is more permissive

### A `depends_on` target behind an inactive profile

A service can declare `profiles:` so it stays out of a default `up`. If another service that *is* being started declares `depends_on` on it, the two declarations contradict each other.

docker compose refuses the project:

```
service "web" depends on undefined service "db": invalid compose project
```

podup activates transitive `depends_on` dependencies even behind inactive profiles; Docker Compose rejects that configuration.

```yaml
services:
  db:
    image: postgres:18-alpine
    profiles: ["debug"]
  web:
    image: nginx:alpine
    depends_on:
      - db
```

Short and long `depends_on` behave alike. For portability, give both services the same profile; with neither selected, both remain inactive.

### `volumes_from`, `links`, and `service:` namespace references

`volumes_from: [data]`, `links: [data:alias]`, and `network_mode` / `ipc` / `pid` / `uts` set to `service:data` add implicit dependencies on the named service. Dependency cycles are rejected.

## Accepted but has no effect

These modeled fields parse but have no effect. The examples are not exhaustive; podup warns about ignored settings.

### Swarm / cluster orchestration

| Field | What it does in Swarm |
|---|---|
| `deploy.mode: global` | Run one replica per cluster node |
| `deploy.placement` | Constrain which nodes a service runs on |
| `deploy.update_config` | Rolling-update parallelism, delay, failure action |
| `deploy.rollback_config` | Automatic rollback behaviour |
| `deploy.endpoint_mode` | VIP vs DNS round-robin load balancing |
| `deploy.restart_policy.delay` / `.window` | No first-class Podman restart delay or attempt-counting window (`condition` and `max_attempts` *are* honored) |
| port long-form `mode:` (`ingress` / `host`) | Swarm ingress routing |

### BuildKit / buildx-only build options

`build.privileged`, `build.ssh`, `build.ulimits`, `build.isolation`, `build.entitlements`, `build.provenance`, `build.sbom`: these have no libpod build-API equivalent and are ignored.

### Windows / Hyper-V-only

`cpu_count`, `cpu_percent`, `credential_spec`, `isolation`: no rootless Podman equivalent.

### Other parsed-but-ignored fields

| Field | Why it has no effect |
|---|---|
| `pull_refresh_after` | strictly validated after interpolation, syntax only; no last-pull clock is available, so it has no runtime effect |
| `attach` | podup follows its own attach/detach logic for `up` log streaming |
| `use_api_socket` | no podup equivalent |
| `provider:` / top-level `models:` | podup runs no model runner; the service/model is not honored |
| volume long-form `driver_config` | not forwarded to Podman |
| `networks.*.enable_ipv4` | Podman networks enable IPv4 by default and expose no toggle |
| `networks.*.ipam.config[].aux_addresses` | not supported by Podman |
| service `networks.*.gw_priority` | not supported by Podman |
| `secrets`/`configs` `driver` / `template_driver` (on non-`external` defs) | external secret-store plugins (Vault, AWS SM, …) podup does not invoke; the secret/config is not staged |

## Not yet supported

| Feature | Status |
|---|---|
| `env_file.format` other than `dotenv` | Any explicit `env_file.format` (including dotenv) emits a warning; the file is still read as dotenv |
| `provider:` / model-runner services (`provider`, top-level `models`) | Modeled and parsed, but not honored; a not-honored warning is emitted |
| Top-level `jobs:` section | Refused: podup runs no job runner, so a file that declares a job is rejected at parse time, before Podman is contacted, with an error naming the first job. An empty or null `jobs:` declares nothing and is accepted |
| Configurable pod exit policy and stop timeout | The pod exit policy stays fixed at `continue`; no pod-level Compose source maps to stop timeout |
| Real-hardware smoke tests on macOS | Pending; the code paths exist but are untested on physical Apple hardware. Windows was validated end-to-end on real hardware (Windows 11, Podman 5.8.3), including the interactive `exec`/`run` path added in 3.0.0. |

Unknown keys warn rather than abort, including in modeled nested objects. Modeled unsupported fields also warn. `extra_hosts` accepts list and mapping forms.

`RUST_LOG=podup=debug` adds lifecycle/network tracing; see the [`RUST_LOG` reference in `commands.md`](commands.md#environment).

## Platforms

podup ships for Linux, macOS and Windows, each x86_64/arm64. Install paths are in the [README](../README.md).

### Engine and transport

podup needs a Podman 5.0 or newer engine. The supported majors are 5.x and 6.x; the libpod client requires the engine's reported major to be at or above `MIN_LIBPOD_API_MAJOR` (5), and the engine version is checked on the first request that contacts Podman. Commands that contact Podman check the version. `config --resolve-image-digests` and `autostart uninstall --purge` also contact it.

The transport is local-only. Only `unix://` (or `npipe://` on Windows) is accepted, from `--socket`, `PODMAN_SOCKET`, `DOCKER_HOST`, or auto-detection; remote `tcp://`/`ssh://`/`http(s)://`/`fd://` schemes are rejected before any connection. This applies to `podman machine` as well. Driving a remote engine over TCP or SSH is not supported; run podup beside the engine.

### Linux

The Linux release binaries are statically linked with musl. The package build is described in [Debian packaging](debian-packaging.md).

Recorded distribution package versions (2026-08-24, with later Ubuntu observations noted):

| distribution        | Podman | podup runs |
|---------------------|--------|------------|
| Debian 12 bookworm  | 4.3.1  | no         |
| Debian 13 trixie    | 5.4.2  | yes        |
| Ubuntu 22.04 LTS    | 3.4.4  | no         |
| Ubuntu 24.04 LTS    | 4.9.3  | no         |
| Ubuntu 26.04 LTS    | 5.7.0  | yes        |
| Fedora 42 and newer | 5.x+   | yes        |

apt requires an installable Podman >=5 package; it does not guarantee a running socket. Tested majors are 5 and 6. Distributions below Podman 5 need another engine source.

### macOS

macOS uses a running rootless `podman machine`'s local Unix socket.

### Windows

Windows uses a running rootless `podman machine`'s local named pipe. The release is not Authenticode-signed; Smart App Control enabled hosts were reported to block it (#1774, 2026-09-22). SmartScreen behavior was not measured. Release-key checks do not establish Windows signing/reputation. For the reported blocked case, run the Linux binary inside `podman-machine-default` using the README's non-apt installer.

In the no-user-systemd WSL case reported on 2026-09-10 (#1778), build `RUN` steps failed until `cgroup_manager` was set to `cgroupfs`. Runtime error and Podman/WSL versions were not recorded. Set it in that user's `containers.conf`:

```toml
# ~/.config/containers/containers.conf
[engine]
cgroup_manager = "cgroupfs"
```

If that file already has an `[engine]` table, add the key to it rather than opening a second one. To check that it took effect, ask the API service:

```sh
podman --remote info --format '{{.Host.CgroupManager}}'
```

If the API still reports `systemd`, restart it. This workaround is for that WSL case, not ordinary Linux.

## File references and path confinement

For path and trusted-input assumptions, see the security model's [Compose files are trusted input](security-model.md#compose-files-are-trusted-input) section.
