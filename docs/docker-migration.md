# Migrating from Docker Compose to podup

Run existing Compose projects on Podman with `podup up`. The sections below cover supported fields, rootless differences and ignored settings.

```sh
cd my-project          # the directory holding docker-compose.yml
podup up
```

## Waiting and exit codes

`wait` waits for containers concurrently and returns the last completed container's code, including zero, matching Docker Compose v2.40.3. Simultaneously ready results follow the order observed by the client. podup prints a table of container names and codes, or NDJSON with `--format json`, instead of Compose's text messages. `--down-project` is not supported. An engine/JSON/connection error remains a failure even if another container later exits zero; outstanding waits are allowed to finish.

`up --exit-code-from SERVICE` uses the triggering container when it belongs to that service. If another service triggers the abort, podup chooses the first observed stopped replica of SERVICE before teardown, with replica order breaking ties; if none has stopped it uses the first replica. This is a minimal approximation of Compose's exit-event ordering: it does not reconstruct the historical order of multiple replicas already stopped when inspected, or monitor later exits during teardown. An unreadable exit status fails with the service name after attempting to stop the project.

## Which logs `up` follows

Like Docker Compose, an attached `up` with services named on the command line follows the logs of those services only. Their `depends_on` services are still started, but their logs are followed only with `--attach-dependencies`. With no service named every service is followed. `--attach SERVICE` replaces the named services as the followed set, `--no-attach SERVICE` removes a service from it, and a service with `attach: false` is never followed. Earlier podup versions followed the dependencies as well; add `--attach-dependencies` to keep that output. `--abort-on-container-exit` reacts to the followed services only.

## Container status filters

`podup ps --status` matches the state or status by case-insensitive prefix: `--status run` matches `running`. Docker Compose compares the whole status. This also applies to podup's `--filter status=...`.

## Works out of the box

`pre_start` accepts a list of full container specifications. During `up`, `run` and `create`, each step inherits unset fields from the service, completes its collections and replaces explicit scalar values. Steps run in order, each in a separate ephemeral container, before the service container is created. A non-zero exit names the service and step number (for example, `service 'web': pre_start step 1: exited with code 7`) and blocks the service and its dependents. Init containers are removed on success or failure. An interrupt (Ctrl-C) can leave the current init container until `down`; restart policies and service hooks do not repeat init work. `create` leaves the service stopped after completing init work. Steps inherit neither ports/expose nor static IP/MAC addresses; a step can declare its own. Steps without their own image or build reuse the service image, including build-only services. A step that declares its own image does not inherit the service's build, and a step that declares its own build does not inherit the service's image, so the step is pulled or built on its own. The one-off settings of a `run` (its command, `--entrypoint`, terminal, ad-hoc mounts and ad-hoc labels) describe the throwaway container and never reach the init steps; the user, working directory and environment do. A failed init leaves the previous service running. An existing running service needs no new init run; `up` completes init work before starting an existing stopped container.

`podup audit` walks the init steps as well: each step is a container of its own and is reported under `<service>/pre_start[i]`, so a step that asks for `privileged: true` or mounts a host path is a finding about that step and not about the service it belongs to.

Quadlet export reports `pre_start is not exported; each init step requires its own container`. No `ExecStartPre=` substitute is emitted, since it cannot preserve the per-step container specifications.

`pre_stop` hooks run inside each running container of the service, in order, before the container is stopped. That happens on `stop`, on `restart`, on `down` and on the final stop of `up` (Ctrl-C, or `--abort-on-container-exit`). A hook is best effort: one that fails, or that cannot run because the container is not running, is logged at debug level and the stop goes ahead. When one level of services cannot be stopped, the levels below it are still told to stop and the first error is what the command reports.

`build.secrets` takes the short form (the name of a top-level secret, also its id in the build) and the long form with `source`, `target`, `uid`, `gid` and `mode`. `target` is the id the Dockerfile reads with `RUN --mount=type=secret,id=...`; it defaults to `source`. `uid`, `gid` and `mode` are accepted and have no effect, because a build secret has no owner or mode to set. Both names may hold only letters, digits, `.`, `_` and `-`.

The following Compose fields are translated onto Podman's libpod API. Fields outside this list still parse (an unknown key never errors, per the Compose spec's forward-compatibility rule) and podup reports any it cannot honor during `up`; see the accepted-but-has-no-effect section.

| Category | Keys |
|---|---|
| Images / build | `image`, `build` (context, dockerfile, args, target, cache_from, cache_to, secrets, labels, network), `pull_policy` (time policies map to `newer`), `pull_refresh_after` (strict syntax only) |
| Execution | `command`, `entrypoint`, `working_dir`, `user`, `umask`, `platform`, `tty`, `stdin_open` |
| Environment | `environment`, `env_file` (dotenv format, or `format: raw`; see the note below the table), variable substitution (`${VAR:-default}`) |
| Networking | `ports`, `expose`, `networks`, `network_mode`, `hostname`, `domainname`, `dns`, `dns_opt`, `dns_search`, `extra_hosts` |
| Volumes | `volumes` (bind, named, tmpfs, npipe; a long-form bind with `create_host_path: false` fails `up` naming the service and the missing path instead of creating it), `tmpfs`, `volumes_from` |
| Secrets / configs | `secrets`, `configs` (file, inline content, environment source, and `external: true` Podman-native secrets) |
| Dependencies | `depends_on` (with `condition:` set to `service_started`, `service_healthy` or `service_completed_successfully`) |
| Health checks | `healthcheck` (test, interval, timeout, retries, start_period, disable) |
| Lifecycle hooks | `pre_start` (ephemeral init containers), `post_start`, `pre_stop` |
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

A memory amount that cannot be used (`mem_limit`, `memswap_limit`, `mem_reservation`, `shm_size`, `deploy.resources.limits.memory`, `deploy.resources.reservations.memory`) is an error that names the service and the key, for example `service 'web': mem_limit "1e100" is not a valid memory amount`. Docker Compose drops such a value and runs the container without the limit; podup refuses to start it. An empty value (`mem_limit: ${MEM:-}` with `MEM` unset) means no limit and is not an error. Valid amounts are a byte count or a number with `b`, `k`, `m`, `g`, `t` or `p` (optionally followed by `b`, or by `ib` as in `512MiB`; any case, all binary multiples, as Docker Compose reads them), and `-1` where the engine takes it.

An `env_file` entry is read as dotenv by default. Unquoted and double-quoted values are interpolated, looking each name up in the project environment first, then in the service's `environment:`, then in the keys set by the env files before it and by the file itself; a single-quoted value stays literal. `format: raw` reads the file line by line with every value kept as written, quotes included, and none interpolated.

`umask` sets the initial process permission mask. Use a string of one to four octal digits, such as `"0022"`, `"022"` or `"22"` (range `0000` to `7777`). Only integer `0` is accepted; quote every other numeric value to avoid YAML radix ambiguity. Validation follows normal interpolation and refuses leftover variables with the service name and a quote hint before container or pod replacement. Normal interpolation can turn numeric text into a YAML integer, so use a leading-zero value such as `0022`. With `config --no-interpolate`, a placeholder in `umask` is refused because the typed field still requires a valid mask. Valid output is padded to four digits. Omitted or null leaves the runtime default; explicit zero sends `"0000"` and masks no permission bits. The quadlet export uses `PodmanArgs=--umask=<value>`, compatible with Podman 5.0. Scalar inheritance, `!reset` and `!override` apply. Explicit `umask: null` changes the configuration hash once on upgrade because the previously unknown null key is now omitted. Tagged payloads use normal interpolation before merge tags are stripped; unresolved masks still fail validation.

`command:` and `entrypoint:` given as a string are split into words the way Docker Compose splits them, and run without a shell: `entrypoint: echo got` with `command: [x, y]` runs `echo got x y`. Whitespace separates words, single and double quotes group, and a backslash keeps the next character (`\n` and `\t` are a newline and a tab). Shell syntax is not interpreted. An unquoted `;`, `&`, `|`, `<` or `>` ends the string and the rest is dropped, as Compose does (`command: make && make install` runs `make`; podup names the dropped part in a warning), and an unquoted parenthesis or a quote that never closes fails the load. For a shell, write the list form (`["sh", "-c", "make && make install"]`) or `sh -c '...'` with the script in quotes. The same rules apply to `run --entrypoint`. A healthcheck `test:` string still runs through `sh -c`.

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

### Bridge network isolation

podup creates every bridge network with the netavark option `isolate=true`, so two bridge networks do not reach each other's unpublished ports, as in Docker. The Quadlet export writes the same default on the `.network` unit as `Options=isolate=true`, so a project started from the exported units is isolated like one started with `podup up`. A compose file that sets `isolate` in the network's `driver_opts` wins, including `"false"` for a project that deliberately spans networks. Networks with another driver are left alone. A network that already exists on the host keeps the options it was created with: after updating the units, remove the old network (`podman network rm <project>_<name>`) so the unit creates it again with `isolate=true`.

### Time-based pull policies

`pull_policy:` accepts `always`, `never`, `missing` (alias `if_not_present`), `build`, Podman's `newer`, `daily`, `weekly`, `every_<n>[wdhms]`, and the podup extension `refresh`.

`daily`, `weekly`, `every_<n>[wdhms]` and `refresh` map to libpod policy `newer`, including Quadlet `Pull=newer`. The registry is checked for a newer image and the local image is used when current. This may use more network than the declared interval. Podman reports image build time, not last-pull time, so podup does not compare image ages. Each service using a time policy emits one warning:

```text
pull_policy "<value>" is treated as "newer": podup does not track when an image was last pulled
```

Warnings name the effective policy after an engine override. Fixed overrides, and `x-podman-autoupdate: registry` on `pull` and `up`, suppress the time-policy warning; `config` and `generate quadlet` still emit it for such a service. Build services use or build their local image. In the Quadlet export only services with a `build:` get `Pull=never` and no time-policy warning; an `image:`-only service keeps its own policy (`Pull=newer` for `daily`) and still warns.

The podup extension `pull_refresh_after` is an optional string, validated for syntax and otherwise unused with every policy. Setting it with `refresh` adds no further effect or warning; with a fixed policy it produces no warning. Its grammar is `([0-9]+[wdhms])+`, and the `every_` policy grammar is `every_([0-9]+[wdhms])+`: ASCII digits, lowercase units, one or more pairs, at most 4294967295 total seconds. Zero, repeated units and any unit order are accepted. Empty counts, unknown units, fractions, subsecond units, signs, whitespace and overflow are refused with the service, original value and accepted values. Validation runs before mutation for `up`, `create`, `run`, `pull`, `build`, standalone `watch`, watch rebuilds and pod recreation, and also covers `config` and the Quadlet export gate. `config --no-interpolate` also validates these two fields strictly: unresolved variables are refused instead of rendered. This limitation avoids accepting malformed values in ordinary `config`.

```yaml
services:
  web:
    image: nginx:1.27
    pull_policy: refresh
    pull_refresh_after: 24h
```

`!reset`, `!override null` and plain null clear an inherited `pull_refresh_after` through `extends` and multiple `-f` files. Services omitting the field retain their serialized configuration. An explicit `pull_refresh_after: null` may change the configuration hash once on upgrade.

With several `-f` files the merge follows `docker compose config`. A volume, network, secret or config declared in more than one file merges key by key, so a second file that only adds a label keeps the first file's `driver`; a list the second file sets, such as `ipam.config`, replaces the first file's. Inside a service, `command: null`, `entrypoint: null` or `!reset` clears the inherited value, and an absent key keeps it. The per-network config of a service (`aliases`, `ipv4_address`, `priority`) merges field by field, with the aliases appended. `build.cache_from`, `build.tags`, `build.extra_hosts` and `deploy.resources.reservations.devices` append the second file's entries without repeats, while a `healthcheck.test` list is still replaced. A different `logging.driver` drops the previous driver's `options`; the same driver merges them. The ports `8080:80` and `8080:80/tcp`, and the mounts `/data` and `/data/`, each count as one entry.

`!reset null` on a top-level entry of `services`, `networks`, `volumes`, `secrets`, `configs` or `models` removes that entry from the files merged before it, as it does for a key inside a service. `!reset` on one entry or sub-key inside a service, such as `depends_on: {db: !reset {}}`, `labels: {x: !reset null}`, `networks: {n1: !reset null}`, `healthcheck: {interval: !reset null}` or `deploy: {resources: !reset {}}`, removes just that part and keeps the rest. `config` prints every declared volume with the name `up` will create, so its output declares each volume a service mounts and reads back as the same project.

### GPU reservations are host-dependent

`deploy.resources.reservations.devices` and the `gpus:` shorthand are honored, but only for **NVIDIA** GPUs, and only when the host exposes them through CDI (the NVIDIA Container Toolkit must be installed and the CDI spec generated). Reservations for other drivers or capabilities are warned about and skipped.

### Depth of `extends` and `include` chains

An `extends` chain and an `include` tree are each followed at most 16 levels deep, and one project reads at most 512 included files; past that podup stops with an error. The limits guard against files that refer to each other and against files written to be read forever, and cycles are reported on their own. docker compose has no such limit (a chain of 60 `extends` loads), so a project that nests deeper than 16 levels loads under docker compose and not under podup.

### Interpolation across compose files

`!override` and `!reset` payloads are interpolated before their merge directives are applied, including nested mappings and lists. External `extends.file` chains receive the referring compose file's interpolation mode and resolved explicit env paths, without rebasing those paths at each external directory. The root file anchors env paths to its compose directory. `config --no-interpolate` preserves placeholders in external bases too, subject to typed-field validation such as `umask` and pull policies. Included files retain their original interpolation context, including local env files, when resolving external bases. Includes are merged with parent definitions winning before same-file inheritance is resolved.

### The project name as a variable

The name the project runs under is available to the file as `${COMPOSE_PROJECT_NAME}`, in every file the project reads (the `-f` files, `include:`, `extends:` and `env_file:`). It is the first of these that supplies one: the `-p` / `--project-name` flag, the `COMPOSE_PROJECT_NAME` environment variable, the top-level `name:`, then the sanitized name of the project directory. With several `-f` files the `name:` of the last file that sets one wins. The `name:` is normalized as docker compose does it (lowercase, characters outside `a-z`, `0-9`, `_` and `-` dropped, leading `_` and `-` stripped, so `My.App v2` is `myappv2`; a name with a non-ASCII character is refused instead, where docker compose would drop the character), while a `-p` or `COMPOSE_PROJECT_NAME` with an uppercase letter is refused. The top-level `name:` is interpolated before the project name exists, so `name: ${COMPOSE_PROJECT_NAME:-fallback}` takes the flag or the variable when there is one and the default otherwise, and `name: ${COMPOSE_PROJECT_NAME}` with none of them is the same error docker gives, an empty project name. A `COMPOSE_PROJECT_NAME` set only in `.env` or an `--env-file` is read by interpolation but does not name the project; use `-p` or the process environment for that. `config --no-interpolate` leaves the placeholder as written.

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

### `tmpfs:` with `uid=` and `gid=`

Podman has no `uid=` or `gid=` option for a tmpfs (`unknown mount option "uid=1000"`). It can give the tmpfs to the user the container runs as, and podup uses that when the pair names the service's own `user:` (`user: "1000:1000"` with `/t:uid=1000,gid=1000`): the options become Podman's `U`. In every other case `up` stops before creating anything and says what to set. A `user:` that is a name, or a number with no group when `gid=` is asked for, is not accepted, because podup cannot tell which group it stands for. `generate quadlet` applies the same rule: the pair becomes `U` in the `Tmpfs=` line or the command stops before writing a unit.

### `network_mode: host`

Shares the Podman host's network namespace, including access to localhost services. Rootless operation still limits process privileges; it does not provide network isolation in this mode.

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

## Refused, and docker compose accepts

These were compared against `docker compose` 5.7.0. In each one podup stops, or acts differently, where docker compose goes on.

### An external network that also sets `driver`, `ipam` or other creation attributes

```yaml
networks:
  shared:
    external: true
    driver: bridge
```

docker compose accepts this file. podup refuses it, naming the network and the attributes: an external network is used as it is, and podup does not apply creation attributes to a network it did not create. Remove the attributes, or drop `external: true` so that podup creates the network.

### More than 64 levels of nested defaults

`${A:-${A:-...}}` nested deeper than 64 levels is refused with "interpolation nesting too deep". docker compose resolves 65 levels without complaint. A file that nests defaults that deep is rare, and the cap keeps the memory of a hostile file bounded.

### `--profile a,b` is two profiles

podup reads a comma in `--profile` (and in `COMPOSE_PROFILES`) as a separator, as its help says. docker compose reads `--profile a,b` as one profile named `a,b`. Pass one `--profile` per profile to get the same result with both.

### The exit code of a usage error

A command line that does not parse (an unknown option, a missing argument) exits with 2. docker compose exits with 1. A script that tests for exactly 1 has to accept 2. Running `podup` with no subcommand exits with 2 too; docker compose exits with 0.

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

A `depends_on` that names a service the file does not define is refused, with its own message (`service 'web' depends on undefined service 'db'; define it, or remove it from 'depends_on'`). That holds whether or not the dependency is `required`, because a name defined nowhere is a typo and not a service that is unavailable. A service that is defined but left out of the run by an inactive profile is not affected by this rule.

### `volumes_from`, `links`, and `service:` namespace references

`volumes_from: [data]`, `links: [data:alias]`, and `network_mode` / `ipc` / `pid` / `uts` set to `service:data` add implicit dependencies on the named service. Dependency cycles are rejected.

### Interpolation reaches the keys of top-level maps

The Compose Specification says interpolation applies to YAML values and not to keys. podup interpolates keys as well, so `networks: {"${NET_NAME}": {}}` names the network from the variable and the file loads. docker compose refuses that file (`networks additional properties '${NET_NAME}' not allowed`). It is a deliberate difference: a file that depends on it runs only under podup, so for portability write the name out and keep variables in values.

### `content:` on a secret

A top-level secret can take `content:` (inline text), as a config can. The specification defines it for configs only, and docker compose refuses it on a secret (`secrets.mysec additional properties 'content' not allowed`). podup accepts it and stores the text as a Podman secret, one of the places compose is extended with what Podman does. A file that uses it does not load under docker compose.

### Jobs and schedules

podup never fires a `schedule` itself; it is not a daemon. `generate quadlet` turns a schedule into a systemd timer, and systemd fires it. An entry is exported only when the timer fires on the same calendar as the cron line and under the same policy, with the daylight saving exception below. Anything else is reported with the job, the entry number and the reason, and nothing is written for it.

Exported: five fields (minute, hour, day of month, month, day of week) made of `*`, numbers, lists, ranges, steps (`*/5`, `10-30/10`) and three-letter month and day names, plus `@hourly`, `@daily`, `@weekly`, `@monthly`, `@yearly` and `@annually`. Sunday is `0` or `7`. A bare `?` is read as `*` in the two day fields. Each field is expanded to the set of values it matches and the `OnCalendar=` text is written from those sets.

Not exported, and why:

| Entry | Reason |
|---|---|
| Day of month and day of week both restricted (`0 0 1 * mon`) | cron fires when either matches, systemd only when both do; no spelling of one gives the other. Restrict one and leave the other `*` |
| `concurrency: queue` | a timer drops a fire while the previous run is active and never queues it |
| `@reboot`, `@midnight`, `L`, `W`, `#`, 6 or 7 fields (seconds, year) | no calendar time, or not the same meaning in systemd |
| A step on a single value (`5/15`), a backwards range (`5-1`, `mon-sun`) | crons read these differently |
| A value out of range, an empty list element | invalid |
| A `timezone` that is not an IANA name | the name goes into the unit text, so only letters, digits and `_ + - /` are accepted |

Daylight saving time is where systemd and cron differ and the timer cannot fix it. In a zone with summer time (and with no `timezone`, which is the host zone), a run set for the hour skipped in spring, such as `30 2 * * *` in `Europe/Madrid` on the last Sunday of March, does not happen that day, and a run set for the hour repeated in autumn can be missed; `Persistent=true` does not make it up. Use `timezone: UTC` or an `Etc/` zone for exact times. `generate quadlet` warns once per job whose exported timers use the host zone or a zone other than `UTC`, `GMT` and `Etc/*`. A `timezone` must be `UTC`, `GMT` or the `Area/Location` form; systemd still rejects a name it does not know when it loads the timer, so check it on the host that runs it. A step wider than the field (`*/60` in minutes, `*/12` in months, `*/31` in days) is not exported even though cron accepts it: podup stays on the safe side. A schedule that is valid but can never fire (`0 0 30 2 *`) is exported with a warning. The timers are plain systemd units and must be installed where systemd reads them, not in the Quadlet directory; see [Jobs in `generate quadlet`](commands-reference.md#jobs-in-generate-quadlet).

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

`build.privileged`, `build.entitlements`, `build.provenance` and `build.sbom` have no `podman build` equivalent: the live build ignores them and the Quadlet export reports them. `build.no_cache_filter` is ignored too. It cannot filter the cache per stage because libpod exposes only the all-or-nothing `nocache` parameter.

`build.ssh` is accepted as a list (`[default, mykey=/path/key]`) or a mapping (`{default: , mykey: /path/key}`), both read as `id[=path]` entries, and is honored by the Quadlet export, which runs `podman build` directly (`PodmanArgs=--ssh`); the live build cannot forward it over the libpod build API and ignores it. `build.ulimits` is honored by both.

`build.isolation` names a container isolation technology (`default`, `process`, `hyperv`), which `podman build` refuses, so the Quadlet export writes `PodmanArgs=--isolation` only for the buildah isolation types (`oci`, `rootless`, `chroot`) and reports any other value; the live build ignores the field.

The `[Build]` keys `Annotation=`, `Volume=` and `DNS=` have no compose `build:` field to feed them (compose `build:` has no `annotations`, `volumes` or `dns` key), so nothing writes them.

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
| `logging.options.tag` with `json-file` or `k8s-file` | Podman's file logger refuses a tag when the container starts; podup drops it with a warning, as it does `max-file` |
| `secrets`/`configs` `driver` / `template_driver` (on non-`external` defs) | external secret-store plugins (Vault, AWS SM, …) podup does not invoke; the secret/config is still staged from its `file`, `content` or `environment` source, and only a definition with none has nothing to stage |

## Not yet supported

| Feature | Status |
|---|---|
| `env_file.format` other than `dotenv` and `raw` | Emits a warning; the file is still read as dotenv. `raw` is supported and is read verbatim |
| `provider:` / model-runner services (`provider`, top-level `models`) | Modeled and parsed, but not honored; a not-honored warning is emitted |
| Top-level `jobs:` `schedule` trigger | The structure is validated and each job with a schedule is reported once. podup does not fire schedules; `generate quadlet` exports them as systemd timers where it can, see [Jobs and schedules](#jobs-and-schedules). `podup run` starts a job by hand (a `manual: false` job is refused, also as a dependency), `podup up` never does. `!reset`/`!override` work on triggers, a job never activates a service behind an inactive profile, and `down` removes job containers; see [Jobs](commands-reference.md#jobs) |
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
