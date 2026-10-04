# Rootless autostart

`podup autostart` keeps a compose stack running across reboots, rootless and user-scope: it writes only under `${XDG_CONFIG_HOME:-~/.config}` and drives everything through `systemctl --user`. Enable lingering so the unit starts at boot without a login.

## Prerequisite: lingering

Lingering starts the user manager and `/run/user/<uid>` at boot, independent of any login. Enable it once as root, then verify:

```bash
sudo loginctl enable-linger appuser
loginctl show-user appuser --property=Linger   # Linger=yes
```

`podup autostart install` warns if lingering is off, because the unit it writes will not start at boot until it is enabled.

## Picking a mode

`podup autostart install --mode` selects the backend. All three are rootless and user-scope.

| Mode | What it installs | Choose it when |
|---|---|---|
| `service` (default) | One `Type=oneshot` unit running `podup up -d --no-build --pull never` at boot, `podman stop` on shutdown. | A simple stack: one unit to enable, one to remove. |
| `quadlet` | One native Podman Quadlet unit per service (`.container`/`.build`/`.volume`/`.network`/`.pod`). Builds run at install and on `autostart rebuild`, not at boot. A service whose build is `build.dockerfile_inline` gets no `.build` unit: Quadlet has no inline-Dockerfile equivalent, so the image must come pre-built under `image:`. | Per-container supervision: systemd restarts, ordering and status for each service. |
| `start` | One `Type=oneshot` unit running `podman start`. Single-service projects only. | Boot resumes an existing container with no compose front-end on the path. |

### Reconcile or restore

`service` and `quadlet` both make the world match the file at boot. Service mode keeps the compose front-end (`.env`, interpolation, profiles) on the runtime path and recreates a container whose `podup.config-hash` label no longer matches, but only from images already on disk. It neither builds nor pulls at boot, so a missing image fails it loudly in `systemctl --user status` and the journal. Quadlet mode renders the stack to systemd units once at install and hands them over; systemd runs containers with no `podup` process in the loop, and Podman still reconciles each `.container` against its unit. Run `podup up -d` once before `autostart install` so the images are present; the default service-mode unit runs `up -d --no-build --pull never`.

`start` does neither. Podman's store survives a reboot, so every setting was baked in earlier. `podman start` restores the lot with no compose file, no `.env`, no registry and no build on the path. Deploy reconciles; boot restores.

### What `start` costs

`start` is single-service only. `podman start` waits for nothing, and a project with `depends_on` (especially `condition: service_healthy`) needs ordering between units, which is what quadlet mode derives from the compose file. `start` refuses a project with more than one service or scaled past one, naming the mode to use instead.

Drift is caught at install, not at boot. `podup` compares the container's `podup.config-hash` label against the file, and over an existing container, refusing a mismatch or a container that does not exist yet. Editing the compose file after installing leaves the unit starting the old container silently; deploy with `podup up -d`.

The three cannot coexist for one project. Reinstalling the same mode rewrites the unit in place; switching modes requires `podup autostart uninstall` first, the same command that removes the update-timer pair left by a service-mode install with `--auto-update`.

## Commands

Status inspects the service/start unit. For Quadlet, use `systemctl --user status <project>-<service>.service` (Quadlet turns `<project>-<service>.container` into `<project>-<service>.service`; substitute the project's name and the service name).

```bash
# install: pick one mode
podup autostart install                          # service mode (default)
podup autostart install --mode quadlet           # quadlet mode
podup autostart install --mode start             # start mode (single-service only)

# install flags
podup autostart install --no-start               # write the unit(s) but do not start yet
podup autostart install --dry-run                # print what would be written/run
podup autostart install --mode service --auto-update daily   # service mode + a sibling timer

# shared operations
podup autostart status                           # this project's unit and session state
podup autostart uninstall                        # remove whichever mode is installed
podup autostart uninstall --purge                # also tear the stack down and drop its volumes

# quadlet only
podup autostart rebuild                          # rebuild every built image + restart
podup autostart rebuild web                      # rebuild just one service
```

## Auto-update

`x-podman-autoupdate` on a service asks Podman's auto-update to keep the image fresh. The three autostart modes pick a different executor:

| Mode | Executor | How it is installed |
|---|---|---|
| `quadlet` | `podman-auto-update.timer` (ships with Podman) | podup writes `AutoUpdate=<value>` on each `.container`. The user must enable `podman-auto-update.timer` on the host for it to fire. |
| `service` | a per-project `<unit>-update.timer` (`hourly`/`daily`/`weekly`) | `podup autostart install --mode service --auto-update <hourly\|daily\|weekly>`. Adds `<unit>-update.service` (oneshot that runs `podup up -d`, which builds and pulls missing images and re-checks the registry for `x-podman-autoupdate: registry` services that have no `build:`) and the timer that fires it; uninstall removes both. |
| `start` | none | the boot path runs `podman start`, not `podup up`. `--auto-update` is rejected with `--mode start`. |

For stacks not under autostart at all, schedule `podup up -d` directly:

```
0 3 * * *  cd /srv/app && podup up -d
```

`uninstall` detects which mode is installed and removes that one; `--mode` is not passed to it. `rebuild` applies to quadlet mode: a Quadlet `.build` unit is `Type=oneshot`, so an image only rebuilds when its build service is restarted.

Quadlet mode builds at install, never at boot or on container restart. The `.container` unit points its `Image=` at the build's tag with `Pull=never`, so Quadlet adds no dependency on the build service. A stack installed by an older `podup` (which pointed the container at `<stem>.build` and let Quadlet re-run the build every start) keeps that behaviour until `podup autostart install --mode quadlet` is run again; `rebuild` is the explicit way to refresh an image in between. Quadlets written without the `# podup-owner:` marker require reinstall or manual removal: `uninstall` skips them.

### Upgrading an existing service-mode install

A unit keeps the `ExecStart` it was written with. Units written by 5.10.5 or earlier run `podup up -d` at boot, which builds or pulls a missing image. After upgrading `podup`, run `podup autostart install` again with the original flags: it overwrites the unit in place and leaves it enabled, so the unit picks up `up -d --no-build --pull never`. `podup autostart install --dry-run` prints the unit it would write.

## Why `--user` and `default.target`

User units wire into `default.target`. `multi-user.target` is a system-manager concept and inert in the user instance, so ordering against it would imply a boot gate that never fires. The same applies to `network-online.target`.

Waiting for the network has to happen somewhere, and Podman ships the piece that makes it possible from a user unit: `podman-user-wait-network-online.service`, a `Type=oneshot` unit that polls `systemctl is-active network-online.target` until the system target comes up. All three modes depend on it; Quadlet mode gets it from Podman's generator (`man podman-systemd.unit`, under *Implicit network dependencies*), which adds `Wants=` and `After=` to every `.container` unit. Service mode writes its final unit itself, with the same two lines:

```ini
[Unit]
Description=podup <project>
Wants=podman-user-wait-network-online.service
After=podman-user-wait-network-online.service
```

The shim ships from Podman 5.3.0; on 5.0-5.2 the dependency is absent, so there is no network ordering. Nothing regresses on those versions; they get no ordering. `autostart status` reports whether the shim is loadable.

## Running `systemctl --user` for a login-less account

For a login-less account with lingering enabled, set `XDG_RUNTIME_DIR` when invoking podup or systemctl. The example assumes `/srv/app/compose.yaml`, its build contexts, and `podup` on the PATH of `appuser` (a privileged install in `/usr/local/bin` or `/usr/bin` is not on that user's PATH):

```bash
uid=$(id -u appuser)
ls -d /run/user/$uid          # exists thanks to lingering

sudo -u appuser env XDG_RUNTIME_DIR=/run/user/$uid \
     podup -f /srv/app/compose.yaml autostart install --mode quadlet
```

Use that environment for invocations without a user session.

## One-time rootless setup

For a dedicated service account the account itself needs the usual rootless Podman groundwork, done once:

- **Subordinate UID/GID ranges.** Ensure the account has entries in `/etc/subuid` and `/etc/subgid` (e.g. `appuser:100000:65536`).
- **`podman system migrate` as the user.** Run it **as the account**, never via `sudo` as root, so the migration writes the account's own storage config:

  ```bash
  sudo -u appuser env XDG_RUNTIME_DIR=/run/user/$(id -u appuser) \
       podman system migrate
  ```

- **The Podman API socket.** `podman` is daemonless and needs no socket, but commands that contact the libpod API need one. Enable the API socket before container operations or starting the service-mode unit:

  ```bash
  sudo -u appuser env XDG_RUNTIME_DIR=/run/user/$(id -u appuser) \
       systemctl --user enable --now podman.socket
  ```

Run `podup up -d` once as the account so the service images are present, then `podup autostart install` to write the unit(s), reload the user manager, and start the stack. A reboot brings it back on its own.