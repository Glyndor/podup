//! Container orchestration engine.
//!
//! Translates a parsed [`ComposeFile`] into Podman API calls via bollard.

mod build;
mod container;
mod health;
mod network;
mod profiles;
mod volume;
mod watch;

use std::collections::HashMap;
use std::path::PathBuf;

use bollard::container::LogOutput;
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::query_parameters::{
    ListContainersOptions, LogsOptions, RemoveContainerOptions, StartContainerOptions,
    StopContainerOptions,
};
use bollard::Docker;
use futures::StreamExt;
use tracing::info;

use crate::compose::types::{ComposeFile, LifecycleHook, Service, ServiceCondition};
use crate::error::{ComposeError, Result};

use profiles::{active_profiles_set, service_in_profiles};

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

pub struct Engine {
    docker: Docker,
    project: String,
    base_dir: PathBuf,
}

impl Engine {
    pub fn new(docker: Docker, project: String) -> Self {
        Self {
            docker,
            project,
            base_dir: std::env::current_dir().unwrap_or_default(),
        }
    }

    pub fn with_base_dir(docker: Docker, project: String, base_dir: PathBuf) -> Self {
        Self {
            docker,
            project,
            base_dir,
        }
    }

    // -----------------------------------------------------------------------
    // Public commands
    // -----------------------------------------------------------------------

    pub async fn up(&self, file: &ComposeFile) -> Result<()> {
        self.up_with_options(file, false, &[], &[], false).await
    }

    pub async fn up_with_options(
        &self,
        file: &ComposeFile,
        _detach: bool,
        active_profiles: &[String],
        target_services: &[String],
        no_recreate: bool,
    ) -> Result<()> {
        let order = crate::compose::resolve_order(file)?;
        let active = active_profiles_set(active_profiles);

        // When target_services is non-empty, restrict the start set to those
        // services plus their transitive dependencies.
        let target_set: Option<std::collections::HashSet<String>> = if target_services.is_empty() {
            None
        } else {
            let mut set = std::collections::HashSet::new();
            let mut stack: Vec<String> = target_services.to_vec();
            while let Some(name) = stack.pop() {
                if !set.insert(name.clone()) {
                    continue;
                }
                if let Some(service) = file.services.get(&name) {
                    for dep in service.depends_on.service_names() {
                        if !set.contains(&dep) {
                            stack.push(dep);
                        }
                    }
                }
            }
            Some(set)
        };

        self.create_networks(file).await?;
        self.create_volumes(file).await?;

        for name in &order {
            if let Some(ref set) = target_set {
                if !set.contains(name) {
                    continue;
                }
            }
            let service = &file.services[name];

            if !service_in_profiles(service, &active) {
                tracing::debug!("skipping {name}: no active profile match");
                continue;
            }

            for dep in service.depends_on.service_names() {
                let condition = service.depends_on.condition_for(&dep);
                let dep_service = match file.services.get(&dep) {
                    Some(s) => s,
                    None => continue,
                };
                if !service_in_profiles(dep_service, &active) {
                    continue;
                }
                let dep_container = self.container_name(&dep, dep_service);

                match condition {
                    ServiceCondition::ServiceStarted => {}
                    ServiceCondition::ServiceHealthy => {
                        if dep_service
                            .healthcheck
                            .as_ref()
                            .map(|h| !h.is_disabled())
                            .unwrap_or(false)
                        {
                            self.wait_healthy(&dep_container, dep_service).await?;
                        } else {
                            tracing::debug!(
                                "{dep} requested service_healthy but has no healthcheck — skipping wait"
                            );
                        }
                    }
                    ServiceCondition::ServiceCompletedSuccessfully => {
                        self.wait_completed(&dep_container).await?;
                    }
                }
            }

            let policy = service.pull_policy.as_deref().unwrap_or("missing");
            match (service.build.is_some(), policy) {
                (true, _) => self.build_service(name, service).await?,
                (false, "never") => {}
                (false, "always") => self.pull_image(service).await?,
                (false, _) => self.pull_image(service).await?,
            }

            let replicas = service
                .scale
                .or(service.deploy.as_ref().and_then(|d| d.replicas))
                .unwrap_or(1) as usize;

            for i in 1..=replicas {
                let container_name = if replicas == 1 {
                    self.container_name(name, service)
                } else {
                    format!("{}-{i}", self.container_name(name, service))
                };
                if no_recreate && self.is_container_running(&container_name).await {
                    info!("{container_name} already running — skipping recreate");
                    continue;
                }
                self.create_and_start(&container_name, name, service, file)
                    .await?;
                self.connect_extra_networks(&container_name, service, file)
                    .await?;
                info!("started {container_name}");

                // Execute post_start lifecycle hooks.
                for hook in &service.post_start {
                    self.run_lifecycle_hook(&container_name, hook).await?;
                }
            }
        }

        Ok(())
    }

    pub async fn down(&self, file: &ComposeFile) -> Result<()> {
        self.down_with_options(file, false).await
    }

    pub async fn down_with_options(&self, file: &ComposeFile, remove_volumes: bool) -> Result<()> {
        let mut order = crate::compose::resolve_order(file)?;
        order.reverse();

        for name in &order {
            let service = &file.services[name];
            for container_name in self.replica_names(name, service) {
                // Execute pre_stop lifecycle hooks before stopping.
                for hook in &service.pre_stop {
                    let _ = self.run_lifecycle_hook(&container_name, hook).await;
                }

                let _ = self
                    .docker
                    .stop_container(
                        &container_name,
                        Some(StopContainerOptions {
                            t: Some(10),
                            ..Default::default()
                        }),
                    )
                    .await;

                let _ = self
                    .docker
                    .remove_container(
                        &container_name,
                        Some(RemoveContainerOptions {
                            force: true,
                            v: remove_volumes,
                            ..Default::default()
                        }),
                    )
                    .await;

                info!("removed {container_name}");
            }
        }

        self.cleanup_temp_dir();
        Ok(())
    }

    pub async fn ps(&self, _file: &ComposeFile) -> Result<()> {
        let label = format!("lynx.compose.project={}", self.project);
        let mut filters: HashMap<String, Vec<String>> = HashMap::new();
        filters.insert("label".to_string(), vec![label]);

        let containers = self
            .docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters: Some(filters),
                ..Default::default()
            }))
            .await?;

        println!("{:<40} {:<30} {:<20}", "NAME", "IMAGE", "STATUS");
        for c in containers {
            let names = c
                .names
                .unwrap_or_default()
                .join(", ")
                .trim_start_matches('/')
                .to_string();
            let image = c.image.unwrap_or_default();
            let status = c.status.unwrap_or_default();
            let ports = c
                .ports
                .unwrap_or_default()
                .iter()
                .map(|p| {
                    format!(
                        "{}:{}->{}",
                        p.ip.as_deref().unwrap_or(""),
                        p.public_port.unwrap_or(0),
                        p.private_port
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            println!("{names:<40} {image:<30} {status:<20} {ports}");
        }

        Ok(())
    }

    pub async fn logs(
        &self,
        file: &ComposeFile,
        service_name: Option<&str>,
        follow: bool,
    ) -> Result<()> {
        let targets: Vec<String> = if let Some(svc) = service_name {
            let service = file
                .services
                .get(svc)
                .ok_or_else(|| ComposeError::ServiceNotFound(svc.into()))?;
            vec![self.container_name(svc, service)]
        } else {
            file.services
                .iter()
                .map(|(n, s)| self.container_name(n, s))
                .collect()
        };

        for container_name in targets {
            let mut stream = self.docker.logs(
                &container_name,
                Some(LogsOptions {
                    stdout: true,
                    stderr: true,
                    follow,
                    ..Default::default()
                }),
            );

            while let Some(msg) = stream.next().await {
                match msg? {
                    LogOutput::StdOut { message } => {
                        print!("{}", String::from_utf8_lossy(&message));
                    }
                    LogOutput::StdErr { message } => {
                        eprint!("{}", String::from_utf8_lossy(&message));
                    }
                    _ => {}
                }
            }
        }

        Ok(())
    }

    pub async fn exec(
        &self,
        file: &ComposeFile,
        service_name: &str,
        cmd: Vec<String>,
    ) -> Result<()> {
        let service = file
            .services
            .get(service_name)
            .ok_or_else(|| ComposeError::ServiceNotFound(service_name.into()))?;
        let container_name = self.container_name(service_name, service);

        let exec_id = self
            .docker
            .create_exec(
                &container_name,
                CreateExecOptions::<String> {
                    cmd: Some(cmd),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    attach_stdin: Some(true),
                    tty: Some(true),
                    ..Default::default()
                },
            )
            .await?
            .id;

        match self.docker.start_exec(&exec_id, None).await? {
            StartExecResults::Attached { mut output, .. } => {
                while let Some(msg) = output.next().await {
                    match msg? {
                        LogOutput::StdOut { message } => {
                            print!("{}", String::from_utf8_lossy(&message));
                        }
                        LogOutput::StdErr { message } => {
                            eprint!("{}", String::from_utf8_lossy(&message));
                        }
                        _ => {}
                    }
                }
            }
            StartExecResults::Detached => {}
        }

        Ok(())
    }

    /// Stream logs from all attached services until Ctrl+C.
    ///
    /// Services with `attach: false` are excluded. Respects the compose spec:
    /// when not detaching, all attached services have their output forwarded.
    pub async fn attach_logs(&self, file: &ComposeFile) -> Result<()> {
        use bollard::query_parameters::LogsOptions;
        use futures::StreamExt;

        let attached: Vec<(String, String)> = file
            .services
            .iter()
            .filter(|(_, s)| s.attach.unwrap_or(true))
            .map(|(name, s)| (name.clone(), self.container_name(name, s)))
            .collect();

        if attached.is_empty() {
            return Ok(());
        }

        let streams: Vec<_> = attached
            .iter()
            .map(|(name, cname)| {
                let prefix = name.clone();
                let mut stream = self.docker.logs(
                    cname,
                    Some(LogsOptions {
                        stdout: true,
                        stderr: true,
                        follow: true,
                        ..Default::default()
                    }),
                );
                async move {
                    while let Some(msg) = stream.next().await {
                        match msg {
                            Ok(LogOutput::StdOut { message }) => {
                                print!("{prefix} | {}", String::from_utf8_lossy(&message));
                            }
                            Ok(LogOutput::StdErr { message }) => {
                                eprint!("{prefix} | {}", String::from_utf8_lossy(&message));
                            }
                            _ => {}
                        }
                    }
                }
            })
            .collect();

        tokio::select! {
            _ = futures::future::join_all(streams) => {}
            _ = tokio::signal::ctrl_c() => {}
        }

        Ok(())
    }

    /// Remove containers that belong to this project but are no longer in the compose file.
    pub async fn remove_orphans(&self, file: &ComposeFile) -> Result<()> {
        let label = format!("lynx.compose.project={}", self.project);
        let mut filters: HashMap<String, Vec<String>> = HashMap::new();
        filters.insert("label".to_string(), vec![label]);

        let running = self
            .docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters: Some(filters),
                ..Default::default()
            }))
            .await?;

        let known: std::collections::HashSet<String> = file
            .services
            .iter()
            .flat_map(|(n, s)| self.replica_names(n, s))
            .collect();

        for c in running {
            let names = c.names.unwrap_or_default();
            for raw in &names {
                let name = raw.trim_start_matches('/');
                if !known.contains(name) {
                    tracing::info!("removing orphan container {name}");
                    let _ = self
                        .docker
                        .remove_container(
                            name,
                            Some(RemoveContainerOptions {
                                force: true,
                                ..Default::default()
                            }),
                        )
                        .await;
                }
            }
        }
        Ok(())
    }

    pub async fn pull(&self, file: &ComposeFile) -> Result<()> {
        let futs: Vec<_> = file
            .services
            .values()
            .filter(|s| s.image.is_some())
            .map(|s| self.pull_image(s))
            .collect();

        let results = futures::future::join_all(futs).await;
        for r in results {
            r?;
        }
        Ok(())
    }

    pub async fn restart(&self, file: &ComposeFile, service_name: Option<&str>) -> Result<()> {
        let names: Vec<String> = if let Some(svc) = service_name {
            if !file.services.contains_key(svc) {
                return Err(ComposeError::ServiceNotFound(svc.into()));
            }
            vec![svc.to_string()]
        } else {
            file.services.keys().cloned().collect()
        };

        for name in &names {
            let service = &file.services[name];
            let container_name = self.container_name(name, service);

            let _ = self
                .docker
                .stop_container(
                    &container_name,
                    Some(StopContainerOptions {
                        t: Some(10),
                        ..Default::default()
                    }),
                )
                .await;

            self.docker
                .start_container(&container_name, None::<StartContainerOptions>)
                .await?;

            info!("restarted {container_name}");

            // Cascade restart to dependents with depends_on.restart: true.
            for (dep_name, dep_service) in &file.services {
                if dep_service.depends_on.restart_for(name) {
                    let dep_container = self.container_name(dep_name, dep_service);
                    let _ = self
                        .docker
                        .stop_container(
                            &dep_container,
                            Some(StopContainerOptions {
                                t: Some(10),
                                ..Default::default()
                            }),
                        )
                        .await;
                    if let Err(e) = self
                        .docker
                        .start_container(&dep_container, None::<StartContainerOptions>)
                        .await
                    {
                        tracing::warn!("cascade restart of {dep_name} failed: {e}");
                    } else {
                        info!("cascade-restarted {dep_container} (depends_on.restart)");
                    }
                }
            }
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Internal
    // -----------------------------------------------------------------------

    async fn run_lifecycle_hook(&self, container_name: &str, hook: &LifecycleHook) -> Result<()> {
        use bollard::exec::{CreateExecOptions, StartExecResults};

        let cmd = hook.command.to_exec();
        let env: Option<Vec<String>> = {
            let m = hook.environment.to_map();
            if m.is_empty() {
                None
            } else {
                Some(
                    m.into_iter()
                        .filter_map(|(k, v)| v.map(|v| format!("{k}={v}")))
                        .collect(),
                )
            }
        };

        let exec_id = self
            .docker
            .create_exec(
                container_name,
                CreateExecOptions::<String> {
                    cmd: Some(cmd),
                    user: hook.user.clone(),
                    privileged: hook.privileged,
                    working_dir: hook.working_dir.clone(),
                    env,
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    ..Default::default()
                },
            )
            .await?
            .id;

        match self.docker.start_exec(&exec_id, None).await? {
            StartExecResults::Attached { mut output, .. } => {
                use bollard::container::LogOutput;
                use futures::StreamExt;
                while let Some(msg) = output.next().await {
                    match msg? {
                        LogOutput::StdOut { message } => {
                            print!("{}", String::from_utf8_lossy(&message));
                        }
                        LogOutput::StdErr { message } => {
                            eprint!("{}", String::from_utf8_lossy(&message));
                        }
                        _ => {}
                    }
                }
            }
            StartExecResults::Detached => {}
        }

        Ok(())
    }

    async fn is_container_running(&self, container_name: &str) -> bool {
        // Use list_containers (not inspect_container) to avoid Bollard
        // deserialization failures when Podman returns "stopped" state.
        let mut filters = HashMap::new();
        filters.insert("name".to_string(), vec![container_name.to_string()]);
        self.docker
            .list_containers(Some(ListContainersOptions {
                all: false,
                filters: Some(filters),
                ..Default::default()
            }))
            .await
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }

    fn container_name(&self, service_name: &str, service: &Service) -> String {
        service
            .container_name
            .clone()
            .unwrap_or_else(|| format!("{}-{}", self.project, service_name))
    }

    /// Return one container name per replica (indexed when scale > 1).
    fn replica_names(&self, service_name: &str, service: &Service) -> Vec<String> {
        let replicas = service
            .scale
            .or(service.deploy.as_ref().and_then(|d| d.replicas))
            .unwrap_or(1) as usize;
        let base = self.container_name(service_name, service);
        if replicas == 1 {
            vec![base]
        } else {
            (1..=replicas).map(|i| format!("{base}-{i}")).collect()
        }
    }
}
