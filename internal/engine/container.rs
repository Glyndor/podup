//! Container creation, configuration, and lifecycle management.
//!
//! [`Engine::create_and_start`] is the main entry point: it assembles the full
//! bollard `Config` from a [`Service`] definition (env vars, mounts, ports,
//! resource limits, networking) and starts the container. Helper functions
//! (`build_env`, `build_mounts`, `resolve_resources`, etc.) each own one
//! slice of the config to keep the mapping manageable.

use std::collections::HashMap;
use std::path::Path;

use bollard::models::{
    ContainerCreateBody, DeviceMapping, DeviceRequest, HealthConfig, HostConfig,
    HostConfigLogConfig, NetworkingConfig, ResourcesBlkioWeightDevice, ResourcesUlimits,
    RestartPolicy as BollardRestart, RestartPolicyNameEnum, ThrottleDevice,
};
use bollard::query_parameters::{
    CreateContainerOptions, RemoveContainerOptions, StartContainerOptions,
};
use tracing::warn;

use crate::compose::types::{
    Command as ComposeCommand, ComposeFile, HealthCheck, LoggingConfig,
    RestartPolicy as ComposeRestart, Service, VolumeMount, VolumeType,
};
use crate::error::{ComposeError, Result};
use crate::{env_file, ports, size};

use super::network::{build_endpoint_settings, resolve_network_mode};
use super::volume::{build_binds, build_mounts};
use super::Engine;

impl Engine {
    pub(super) async fn create_and_start(
        &self,
        container_name: &str,
        service_name: &str,
        service: &Service,
        file: &ComposeFile,
    ) -> Result<()> {
        let image = service
            .image
            .as_deref()
            .ok_or_else(|| ComposeError::NoImageOrBuild(service_name.into()))?;

        let env = build_env(service, &self.base_dir)?;

        let binds = build_binds(service, &self.base_dir);
        let secret_binds = self.build_secret_binds(service, file)?;
        let config_binds = self.build_config_binds(service, file)?;
        let all_binds: Vec<String> = binds
            .into_iter()
            .chain(secret_binds)
            .chain(config_binds)
            .collect();

        let parsed_ports = ports::parse_ports(&service.ports)?;
        let (port_bindings, exposed_ports_map) = ports::to_bollard(&parsed_ports);

        let mut exposed_port_keys: Vec<String> = exposed_ports_map.into_keys().collect();
        for raw in &service.expose {
            let key = if raw.contains('/') {
                raw.clone()
            } else {
                format!("{raw}/tcp")
            };
            if !exposed_port_keys.contains(&key) {
                exposed_port_keys.push(key);
            }
        }

        let restart_policy = build_restart_policy(service);
        let log_config = build_log_config(service.logging.as_ref());
        let (network_mode, first_network) = resolve_network_mode(service, file);
        let label_file_labels = build_label_file_labels(service, &self.base_dir);

        let mut labels = service.labels.to_map();
        // Merge label_file labels (lower priority than inline labels).
        for (k, v) in label_file_labels {
            labels.entry(k).or_insert(v);
        }
        // Merge deploy.labels (lower priority than service.labels).
        if let Some(deploy) = &service.deploy {
            for (k, v) in deploy.labels.to_map() {
                labels.entry(k).or_insert(v);
            }
        }
        for (k, v) in service.annotations.to_map() {
            labels.insert(format!("annotation.{k}"), v);
        }
        labels.insert("lynx.compose.project".to_string(), self.project.clone());
        labels.insert("lynx.compose.service".to_string(), service_name.to_string());

        let ulimits: Vec<ResourcesUlimits> = service
            .ulimits
            .iter()
            .map(|(name, cfg)| ResourcesUlimits {
                name: Some(name.clone()),
                soft: Some(cfg.soft()),
                hard: Some(cfg.hard()),
            })
            .collect();

        let sysctls: HashMap<String, String> = service.sysctls.to_map();
        let extra_hosts: Vec<String> = service.extra_hosts.clone();
        let dns = service.dns.to_list();
        let dns_search = service.dns_search.to_list();
        let dns_opt = service.dns_opt.to_list();

        let devices: Vec<DeviceMapping> = service
            .devices
            .iter()
            .map(|s| parse_device(s.as_str()))
            .collect();

        let device_requests = build_device_requests(service);

        let tmpfs_list = service.tmpfs.to_list();
        let mut tmpfs_map: HashMap<String, String> =
            tmpfs_list.into_iter().map(|p| (p, String::new())).collect();
        for v in &service.volumes {
            if let VolumeMount::Long {
                volume_type: VolumeType::Tmpfs,
                target,
                tmpfs,
                ..
            } = v
            {
                let opts = tmpfs_options_to_string(tmpfs.as_ref());
                tmpfs_map.insert(target.clone(), opts);
            }
        }

        let (
            mem_limit,
            mem_reservation,
            memswap,
            nano_cpus,
            cpu_quota_eff,
            cpu_period_eff,
            pids_limit,
        ) = resolve_resources(service);

        let blkio = build_blkio_config(service);

        let mut all_links: Vec<String> = service.links.clone();
        all_links.extend_from_slice(&service.external_links);

        let mounts = build_mounts(service);

        let host_config = HostConfig {
            binds: opt_vec(all_binds),
            mounts: if mounts.is_empty() {
                None
            } else {
                Some(mounts)
            },
            network_mode: network_mode.clone(),
            restart_policy,
            port_bindings: opt_map(port_bindings),
            cap_add: opt_vec(service.cap_add.clone()),
            cap_drop: opt_vec(service.cap_drop.clone()),
            sysctls: opt_map(sysctls),
            ulimits: if ulimits.is_empty() {
                None
            } else {
                Some(ulimits)
            },
            extra_hosts: opt_vec(extra_hosts),
            dns: opt_vec(dns),
            dns_search: opt_vec(dns_search),
            dns_options: opt_vec(dns_opt),
            init: service.init,
            privileged: service.privileged,
            log_config,
            pid_mode: service.pid.clone(),
            ipc_mode: service.ipc.clone(),
            uts_mode: service.uts.clone(),
            cgroup_parent: service.cgroup_parent.clone(),
            cgroupns_mode: service.cgroup.as_deref().and_then(|v| v.parse().ok()),
            shm_size: service.shm_size.as_deref().and_then(size::parse_memory),
            userns_mode: service.userns_mode.clone(),
            security_opt: opt_vec(service.security_opt.clone()),
            readonly_rootfs: service.read_only,
            devices: opt_vec(devices),
            device_cgroup_rules: opt_vec(service.device_cgroup_rules.clone()),
            tmpfs: opt_map(tmpfs_map),
            volumes_from: opt_vec(service.volumes_from.clone()),
            links: opt_vec(all_links),
            runtime: service.runtime.clone(),
            memory: mem_limit,
            memory_reservation: mem_reservation,
            memory_swap: memswap,
            memory_swappiness: service.mem_swappiness,
            nano_cpus,
            cpu_shares: service.cpu_shares.map(|s| s as i64),
            cpu_quota: cpu_quota_eff,
            cpu_period: cpu_period_eff,
            cpuset_cpus: service.cpuset.clone(),
            pids_limit,
            cpu_count: service.cpu_count,
            cpu_percent: service.cpu_percent,
            cpu_realtime_period: service.cpu_rt_period,
            cpu_realtime_runtime: service.cpu_rt_runtime,
            oom_kill_disable: service.oom_kill_disable,
            oom_score_adj: service.oom_score_adj,
            storage_opt: opt_map(service.storage_opt.clone()),
            group_add: opt_vec(service.group_add.clone()),
            blkio_weight: blkio.as_ref().and_then(|b| b.weight),
            blkio_weight_device: blkio.as_ref().and_then(|b| b.weight_device.clone()),
            blkio_device_read_bps: blkio.as_ref().and_then(|b| b.device_read_bps.clone()),
            blkio_device_write_bps: blkio.as_ref().and_then(|b| b.device_write_bps.clone()),
            blkio_device_read_iops: blkio.as_ref().and_then(|b| b.device_read_iops.clone()),
            blkio_device_write_iops: blkio.as_ref().and_then(|b| b.device_write_iops.clone()),
            device_requests: if device_requests.is_empty() {
                None
            } else {
                Some(device_requests)
            },
            annotations: opt_map(service.annotations.to_map()),
            ..Default::default()
        };

        let cmd = service.command.as_ref().map(|c| c.to_exec());
        let entrypoint = service.entrypoint.as_ref().map(|c| c.to_exec());

        let networking_config = first_network.as_ref().map(|net| {
            let mut endpoints = HashMap::new();
            let svc_net_cfg = service.networks.config_for(net);
            endpoints.insert(net.clone(), build_endpoint_settings(svc_net_cfg, file));
            NetworkingConfig {
                endpoints_config: Some(endpoints),
            }
        });

        let healthcheck = service.healthcheck.as_ref().map(build_healthcheck);

        let config = ContainerCreateBody {
            image: Some(image.to_string()),
            env: opt_vec(env),
            cmd,
            entrypoint,
            host_config: Some(host_config),
            labels: opt_map(labels),
            exposed_ports: opt_vec(exposed_port_keys),
            tty: service.tty,
            open_stdin: service.stdin_open,
            user: service.user.clone(),
            working_dir: service.working_dir.clone(),
            stop_signal: service.stop_signal.clone(),
            stop_timeout: service
                .stop_grace_period
                .as_deref()
                .and_then(size::parse_duration_secs)
                .map(|s| s as i64),
            hostname: service.hostname.clone(),
            domainname: service.domainname.clone(),
            networking_config,
            healthcheck,
            ..Default::default()
        };

        // Remove any pre-existing container with the same name.
        let _ = self
            .docker
            .remove_container(
                container_name,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;

        self.docker
            .create_container(
                Some(CreateContainerOptions {
                    name: Some(container_name.to_string()),
                    platform: service.platform.clone().unwrap_or_default(),
                }),
                config,
            )
            .await?;

        self.docker
            .start_container(container_name, None::<StartContainerOptions>)
            .await?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Container config helpers
// ---------------------------------------------------------------------------

fn build_env(service: &Service, base_dir: &Path) -> Result<Vec<String>> {
    let entries = service.env_file.to_entries();
    let env_file_vars = if !entries.is_empty() {
        env_file::load_env_file_entries(&entries, base_dir)?
    } else {
        HashMap::new()
    };
    Ok(env_file::merge_env(
        service.environment.to_map(),
        env_file_vars,
    ))
}

pub(crate) fn build_restart_policy(service: &Service) -> Option<BollardRestart> {
    if let Some(r) = &service.restart {
        return Some(match r {
            ComposeRestart::No => BollardRestart {
                name: Some(RestartPolicyNameEnum::NO),
                maximum_retry_count: None,
            },
            ComposeRestart::Always => BollardRestart {
                name: Some(RestartPolicyNameEnum::ALWAYS),
                maximum_retry_count: None,
            },
            ComposeRestart::OnFailure { max_attempts } => BollardRestart {
                name: Some(RestartPolicyNameEnum::ON_FAILURE),
                maximum_retry_count: max_attempts.map(|n| n as i64),
            },
            ComposeRestart::UnlessStopped => BollardRestart {
                name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                maximum_retry_count: None,
            },
        });
    }
    // Fall back to deploy.restart_policy when service.restart is absent.
    // delay/window are Swarm-specific and have no container API equivalent.
    if let Some(drp) = service
        .deploy
        .as_ref()
        .and_then(|d| d.restart_policy.as_ref())
    {
        let name = match drp.condition.as_deref().unwrap_or("any") {
            "none" => RestartPolicyNameEnum::NO,
            "on-failure" => RestartPolicyNameEnum::ON_FAILURE,
            _ => RestartPolicyNameEnum::UNLESS_STOPPED,
        };
        return Some(BollardRestart {
            name: Some(name),
            maximum_retry_count: drp.max_attempts.map(|n| n as i64),
        });
    }
    None
}

fn build_log_config(logging: Option<&LoggingConfig>) -> Option<HostConfigLogConfig> {
    logging.map(|l| HostConfigLogConfig {
        typ: l.driver.clone(),
        config: if l.options.is_empty() {
            None
        } else {
            Some(l.options.clone())
        },
    })
}

fn build_healthcheck(hc: &HealthCheck) -> HealthConfig {
    if hc.is_disabled() {
        return HealthConfig {
            test: Some(vec!["NONE".to_string()]),
            ..Default::default()
        };
    }
    let test = hc.test.as_ref().map(|cmd| match cmd {
        ComposeCommand::Shell(s) => vec!["CMD-SHELL".to_string(), s.clone()],
        ComposeCommand::Exec(v) => v.clone(),
    });
    HealthConfig {
        test,
        interval: hc.interval.as_deref().and_then(size::parse_duration_nanos),
        timeout: hc.timeout.as_deref().and_then(size::parse_duration_nanos),
        retries: hc.retries.map(|r| r as i64),
        start_period: hc
            .start_period
            .as_deref()
            .and_then(size::parse_duration_nanos),
        start_interval: hc
            .start_interval
            .as_deref()
            .and_then(size::parse_duration_nanos),
    }
}

#[allow(clippy::type_complexity)]
fn resolve_resources(
    service: &Service,
) -> (
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
) {
    let mut memory = service.mem_limit.as_deref().and_then(size::parse_memory);
    let mut mem_reservation = service
        .mem_reservation
        .as_deref()
        .and_then(size::parse_memory);
    let memswap = service
        .memswap_limit
        .as_deref()
        .and_then(size::parse_memory);
    let mut nano_cpus = service.cpus.as_deref().and_then(size::parse_cpus);
    let cpu_quota = service.cpu_quota;
    let cpu_period = service.cpu_period.map(|p| p as i64);
    let mut pids_limit = service.pids_limit;

    if let Some(deploy) = &service.deploy {
        if let Some(res) = &deploy.resources {
            if let Some(limits) = &res.limits {
                if memory.is_none() {
                    memory = limits.memory.as_deref().and_then(size::parse_memory);
                }
                if nano_cpus.is_none() {
                    nano_cpus = limits.cpus.as_deref().and_then(size::parse_cpus);
                }
                if pids_limit.is_none() {
                    pids_limit = limits.pids.map(|p| p as i64);
                }
            }
            if let Some(reserv) = &res.reservations {
                if mem_reservation.is_none() {
                    mem_reservation = reserv.memory.as_deref().and_then(size::parse_memory);
                }
            }
        }
    }

    (
        memory,
        mem_reservation,
        memswap,
        nano_cpus,
        cpu_quota,
        cpu_period,
        pids_limit,
    )
}

pub(crate) fn parse_device(s: &str) -> DeviceMapping {
    let parts: Vec<&str> = s.splitn(3, ':').collect();
    let host = parts.first().copied().unwrap_or("").to_string();
    let cont = parts
        .get(1)
        .copied()
        .map(|c| c.to_string())
        .unwrap_or_else(|| host.clone());
    let perm = parts.get(2).copied().unwrap_or("rwm").to_string();
    DeviceMapping {
        path_on_host: Some(host),
        path_in_container: Some(cont),
        cgroup_permissions: Some(perm),
    }
}

pub(crate) fn tmpfs_options_to_string(
    opts: Option<&crate::compose::types::TmpfsOptions>,
) -> String {
    let opts = match opts {
        Some(o) => o,
        None => return String::new(),
    };
    let mut parts: Vec<String> = Vec::new();
    if let Some(size) = opts.size {
        parts.push(format!("size={size}"));
    }
    if let Some(mode) = opts.mode {
        parts.push(format!("mode={mode:o}"));
    }
    parts.join(",")
}

pub(crate) fn opt_vec<T>(v: Vec<T>) -> Option<Vec<T>> {
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

pub(crate) fn opt_map<K, V>(m: HashMap<K, V>) -> Option<HashMap<K, V>> {
    if m.is_empty() {
        None
    } else {
        Some(m)
    }
}

fn build_label_file_labels(service: &Service, base_dir: &Path) -> HashMap<String, String> {
    let mut labels = HashMap::new();
    for path in service.label_file.to_list() {
        let full = if std::path::Path::new(&path).is_absolute() {
            std::path::PathBuf::from(&path)
        } else {
            base_dir.join(&path)
        };
        let Ok(content) = std::fs::read_to_string(&full) else {
            warn!("label_file: cannot read {}", full.display());
            continue;
        };
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let mut parts = trimmed.splitn(2, '=');
            let key = parts.next().unwrap_or("").trim().to_string();
            let val = parts.next().unwrap_or("").to_string();
            if !key.is_empty() {
                labels.insert(key, val);
            }
        }
    }
    labels
}

struct BlkioHostConfig {
    weight: Option<u16>,
    weight_device: Option<Vec<ResourcesBlkioWeightDevice>>,
    device_read_bps: Option<Vec<ThrottleDevice>>,
    device_write_bps: Option<Vec<ThrottleDevice>>,
    device_read_iops: Option<Vec<ThrottleDevice>>,
    device_write_iops: Option<Vec<ThrottleDevice>>,
}

fn build_device_requests(service: &Service) -> Vec<DeviceRequest> {
    use crate::compose::types::CountOrAll;

    let mut requests: Vec<DeviceRequest> = Vec::new();

    // Top-level `gpus:` shorthand.
    if let Some(gpus) = &service.gpus {
        requests.push(DeviceRequest {
            driver: Some("".into()),
            count: Some(gpus.to_count()),
            device_ids: None,
            capabilities: Some(vec![vec!["gpu".into()]]),
            options: None,
        });
    }

    // `deploy.resources.reservations.devices`.
    if let Some(deploy) = &service.deploy {
        if let Some(resources) = &deploy.resources {
            if let Some(reservations) = &resources.reservations {
                for dev in &reservations.devices {
                    if dev.capabilities.is_empty() {
                        continue;
                    }

                    let count = if !dev.device_ids.is_empty() {
                        None
                    } else {
                        Some(
                            dev.count
                                .as_ref()
                                .map(|c: &CountOrAll| c.to_i64())
                                .unwrap_or(-1),
                        )
                    };

                    let device_ids = if dev.device_ids.is_empty() {
                        None
                    } else {
                        Some(dev.device_ids.clone())
                    };

                    requests.push(DeviceRequest {
                        driver: dev.driver.clone().or(Some("".into())),
                        count,
                        device_ids,
                        capabilities: Some(vec![dev.capabilities.clone()]),
                        options: if dev.options.is_empty() {
                            None
                        } else {
                            Some(dev.options.clone())
                        },
                    });
                }
            }
        }
    }

    requests
}

fn build_blkio_config(service: &Service) -> Option<BlkioHostConfig> {
    use crate::compose::types::BlkioConfig;
    let cfg: &BlkioConfig = service.blkio_config.as_ref()?;

    let weight_device = if cfg.weight_device.is_empty() {
        None
    } else {
        Some(
            cfg.weight_device
                .iter()
                .map(|d| ResourcesBlkioWeightDevice {
                    path: Some(d.path.clone()),
                    weight: Some(d.weight as usize),
                })
                .collect(),
        )
    };

    let to_throttle = |devs: &[crate::compose::types::BlkioRateDevice]| {
        if devs.is_empty() {
            None
        } else {
            Some(
                devs.iter()
                    .map(|d| ThrottleDevice {
                        path: Some(d.path.clone()),
                        rate: Some(d.rate_value()),
                    })
                    .collect(),
            )
        }
    };

    Some(BlkioHostConfig {
        weight: cfg.weight,
        weight_device,
        device_read_bps: to_throttle(&cfg.device_read_bps),
        device_write_bps: to_throttle(&cfg.device_write_bps),
        device_read_iops: to_throttle(&cfg.device_read_iops),
        device_write_iops: to_throttle(&cfg.device_write_iops),
    })
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::{parse_device, tmpfs_options_to_string};
    use crate::compose::types::TmpfsOptions;

    #[test]
    fn parse_device_host_container_perm() {
        let d = parse_device("/dev/sda:/dev/xvda:rwm");
        assert_eq!(d.path_on_host.as_deref(), Some("/dev/sda"));
        assert_eq!(d.path_in_container.as_deref(), Some("/dev/xvda"));
        assert_eq!(d.cgroup_permissions.as_deref(), Some("rwm"));
    }

    #[test]
    fn parse_device_default_perm() {
        let d = parse_device("/dev/null:/dev/null");
        assert_eq!(d.cgroup_permissions.as_deref(), Some("rwm"));
    }

    #[test]
    fn parse_device_same_path_both_sides() {
        let d = parse_device("/dev/dri");
        assert_eq!(d.path_on_host.as_deref(), Some("/dev/dri"));
        assert_eq!(d.path_in_container.as_deref(), Some("/dev/dri"));
    }

    #[test]
    fn tmpfs_options_empty() {
        let s = tmpfs_options_to_string(None);
        assert!(s.is_empty());
    }

    #[test]
    fn tmpfs_options_size_only() {
        let opts = TmpfsOptions {
            size: Some(67108864),
            mode: None,
        };
        let s = tmpfs_options_to_string(Some(&opts));
        assert_eq!(s, "size=67108864");
    }

    #[test]
    fn tmpfs_options_mode_only() {
        let opts = TmpfsOptions {
            size: None,
            mode: Some(0o1755),
        };
        let s = tmpfs_options_to_string(Some(&opts));
        assert_eq!(s, "mode=1755");
    }

    #[test]
    fn tmpfs_options_size_and_mode() {
        let opts = TmpfsOptions {
            size: Some(1024),
            mode: Some(0o755),
        };
        let s = tmpfs_options_to_string(Some(&opts));
        assert!(s.contains("size=1024"));
        assert!(s.contains("mode=755"));
    }
}
