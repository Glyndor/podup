# Migrating from Docker Compose to podup

This guide covers what to expect when you switch a `docker-compose.yml` from
Docker to podup on rootless Podman.  The short answer is: **most files work
without any changes**.  Read on for the full picture.

## What works out of the box

Every core Compose spec key is supported:

| Category | Keys |
|---|---|
| Images / build | `image`, `build` (context, dockerfile, args, target, cache_from, cache_to, secrets, labels, network) |
| Execution | `command`, `entrypoint`, `working_dir`, `user`, `platform`, `tty`, `stdin_open` |
| Environment | `environment`, `env_file` (dotenv format), variable substitution (`${VAR:-default}`) |
| Networking | `ports`, `expose`, `networks`, `network_mode`, `hostname`, `domainname`, `dns`, `dns_search`, `extra_hosts` |
| Volumes | `volumes` (bind, named, tmpfs, npipe), `tmpfs`, `volumes_from` |
| Secrets / configs | `secrets`, `configs` (file, inline content, environment source, and `external: true` Podman-native secrets) |
| Dependencies | `depends_on` (with `condition:` — `service_started`, `service_healthy`, `service_completed_successfully`) |
| Health checks | `healthcheck` (test, interval, timeout, retries, start_period, disable) |
| Lifecycle hooks | `post_start`, `pre_stop` |
| Restart policies | `restart`, `deploy.restart_policy` |
| Replicas / scale | `deploy.replicas`, `scale:` |
| Resource limits | `deploy.resources`, `mem_limit`, `cpus`, `cpu_shares`, `cpu_quota`, `pids_limit`, `ulimits`, `blkio_config` |
| Devices | `devices`, `device_cgroup_rules`, `deploy.resources.reservations.devices` (GPU) |
| Security | `cap_add`, `cap_drop`, `security_opt`, `read_only`, `privileged`, `userns_mode` |
| Namespaces | `pid`, `ipc`, `uts`, `cgroup`, `shm_size` |
| Metadata | `labels`, `annotations`, `container_name`, `profiles` |
| Logging | `logging` (driver + options) |
| Compose features | `extends`, `include`, YAML anchors, x-extensions, `develop.watch` |

### External secrets and configs

A secret or config declared `external: true` is mounted from an existing
Podman secret rather than from a file in the project tree — the recommended
pattern for production credentials, since the secret material never lands in a
bind-mounted file. Create the secret before running, exactly as you would with
`docker secret`:

```sh
printf '%s' "$DB_PASSWORD" | podman secret create db_password -
```

```yaml
services:
  db:
    image: postgres:16
    secrets:
      - db_password
secrets:
  db_password:
    external: true
```

The secret appears at `/run/secrets/db_password` (configs use the long-form
`target:` path). If the named Podman secret does not exist, `podup up` fails
fast rather than starting a container without it. Use a top-level `name:` when
the Podman secret is named differently from the compose reference.

## Rootless Podman differences

These are Podman behaviours, not podup limitations.  They apply equally to any
Podman-based tool.

### Privileged ports (< 1024)

Rootless containers cannot bind host ports below 1024 unless the kernel allows
it:

```bash
# Allow a single port (temporary, requires root once):
sudo sysctl net.ipv4.ip_unprivileged_port_start=80

# Or map through a higher port in your compose file:
ports:
  - "8080:80"
```

### UID/GID mapping

Containers run as your host user's UID inside a user namespace.  If a container
image writes files with UID 0 (root inside the container), those files appear
owned by your user on the host.  Bind-mount permissions reflect your host
user's access.

### Volume SELinux labels

On SELinux-enforcing systems, bind mounts require relabeling.  Append `:z`
(shared) or `:Z` (private) to the volume spec:

```yaml
volumes:
  - ./data:/app/data:Z
```

### `network_mode: host`

Attaches the container to your user's network namespace, not a privileged host
namespace.  Traffic is still limited to your user's capabilities.

### `network_mode: none`

Supported.  The container gets a loopback interface only.

## Deprecated fields (honored with a warning)

### `mac_address:` at the service level

The Compose spec deprecated the top-level `mac_address` field in favour of
per-network configuration.  podup still honours it (for backward compatibility)
and applies it to the primary network, but logs a deprecation warning.

**Migration:** move it under `networks:`:

```yaml
# before
services:
  web:
    mac_address: "02:42:ac:11:00:02"

# after
services:
  web:
    networks:
      default:
        mac_address: "02:42:ac:11:00:02"
```

## Swarm-only fields (accepted, no effect)

These fields are part of the Compose spec for Docker Swarm deployments.  They
are parsed without error so existing compose files validate cleanly, but they
have no equivalent in single-host rootless Podman.  podup logs a warning for
each one that is present.

| Field | What it does in Swarm |
|---|---|
| `deploy.mode: global` | Run one replica per cluster node |
| `deploy.placement` | Constrain which nodes a service runs on |
| `deploy.update_config` | Rolling-update parallelism, delay, failure action |
| `deploy.rollback_config` | Automatic rollback behaviour |
| `deploy.endpoint_mode` | VIP vs DNS round-robin load balancing |

If you see warnings for these fields, you can safely remove them from your
compose file when targeting a single-host Podman deployment.

## Nothing is dropped silently

podup follows the Compose spec's forward-compatibility rule — an unknown key is
never a hard error, so `x-*` extensions and newer compose fields keep parsing.
But anything podup cannot translate is **reported**, never silently ignored, so
a typo or an unmapped feature can't hide. At parse time podup warns about:

- unknown keys at the top level and inside every modeled object (services and
  their `healthcheck`, `deploy`, `develop.watch`; top-level `networks`,
  `networks.*.ipam`, and `volumes`) — usually a typo such as `enviroment:`;
- fields it models but cannot honor on rootless Podman: `cpu_count` and
  `cpu_percent` (Windows/Hyper-V only), `networks.*.enable_ipv4`, and the
  BuildKit-only `build.privileged`, `build.ulimits`, `build.isolation`,
  `build.entitlements`, `build.provenance`, `build.sbom`.

This is what lets a compose file written for a newer Docker or Podman release
run under podup: unsupported additions surface as warnings instead of vanishing.

`extra_hosts` is accepted in both the list (`["host:ip"]`) and mapping
(`{host: ip}`) forms.

## Not yet supported

| Feature | Status |
|---|---|
| `env_file.format` values other than `dotenv` | Error is emitted; only the `dotenv` format is accepted |
| `gpus:` as a list of device objects | Use `deploy.resources.reservations.devices` for GPU reservations; the scalar `gpus: all` / `gpus: N` shorthand is supported |
| `provider:` / model-runner services (`provider`, top-level `models`) | No rootless Podman equivalent; reported as an unknown key |
| Real-hardware smoke tests on macOS and Windows | Pending ([#48](https://github.com/Glyndor/podup/issues/48)); code paths exist but are untested on physical hardware |

## Enabling verbose output

Run with `RUST_LOG=podup=debug` to see network creation, container lifecycle
events, and the parse-time warnings podup emits for any compose field it cannot
translate. See the [`RUST_LOG` reference in `commands.md`](commands.md#environment)
for the full level table.

## Quick compatibility checklist

Before running `podup up` on an existing compose file:

- [ ] Remove or remap any host ports below 1024 if running fully rootless.
- [ ] Add `:Z` to bind mounts on SELinux hosts if containers cannot write to them.
- [ ] Check for `mac_address:` at the service level — move to `networks:` to silence the warning.
- [ ] Check for Swarm-only `deploy.*` fields — they are harmless but can be cleaned up.
- [ ] Verify `env_file` uses dotenv format (key=value lines, `#` comments).
