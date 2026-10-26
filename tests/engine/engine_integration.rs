//! Integration tests that exercise the engine against a real Podman daemon.
//!
//! I require a reachable Podman for the user namespace assertions. Other groups
//! skip when Podman is unavailable unless `PODUP_REQUIRE_PODMAN=1` is set.
//! I use that setting in the nested-virt lane so an unreachable Podman fails.
//!
//! The test bodies are split across the `engine_integration/` submodules to
//! keep each file under the source line limit, one behaviour per module. The
//! helpers they all share live in `engine_integration/support/harness.rs` and
//! `engine_integration/support/pollers.rs`, re-exported here so every submodule
//! reaches them the same way it always has, through `use super::*;`.
//!
//! The files sit in one folder per concern, grouped the way the engine is
//! (`workload/`, `images/`, `objects/`, `observe/`, `host_files/`, with `cli/`
//! split again by command family), but every module
//! is still declared here with a `#[path]`, so each test keeps the name
//! `engine_integration::<module>::<test>` whichever folder holds its file.
use std::fs;

use podup::{parse_files_with_env_files, parse_str, Client, Engine};

#[path = "engine_integration/support/harness.rs"]
mod harness;
#[path = "engine_integration/support/pollers.rs"]
mod pollers;
pub(crate) use harness::*;
pub(crate) use pollers::*;

// ---------------------------------------------------------------------------
// Test groups (see engine_integration/*.rs)
// ---------------------------------------------------------------------------

#[path = "engine_integration/autostart/autostart_quadlet.rs"]
mod autostart_quadlet;
// Unix only: autostart is systemd, and the image checks reach the Podman
// socket by its `/run/user/<uid>` path.
#[cfg(unix)]
#[path = "engine_integration/autostart/autostart_service_boot.rs"]
mod autostart_service_boot;
#[path = "engine_integration/images/build/build_cache_invalidation.rs"]
mod build_cache_invalidation;
#[path = "engine_integration/images/build/build_images.rs"]
mod build_images;
// Unix only: it reaches the Podman socket by its `/run/user/<uid>` path.
#[cfg(unix)]
#[path = "engine_integration/images/build/build_labels.rs"]
mod build_labels;
#[path = "engine_integration/images/build/build_resources.rs"]
mod build_resources;
// Unix only: the assertion lists external containers via `podman ps -a
// --external` against a Unix-domain socket, the path the libpod client
// drives.
#[path = "engine_integration/observe/logs/attach_logs_timestamps.rs"]
mod attach_logs_timestamps;
#[cfg(unix)]
#[path = "engine_integration/images/build/build_failure_cleanup.rs"]
mod build_failure_cleanup;
#[path = "engine_integration/images/build/build_sparse_context.rs"]
mod build_sparse_context;
#[path = "engine_integration/host_files/copy/cp_flags.rs"]
mod cp_flags;
#[path = "engine_integration/host_files/copy/cp_transfer.rs"]
mod cp_transfer;
#[path = "engine_integration/objects/network/explicit_network.rs"]
mod explicit_network;
#[path = "engine_integration/objects/resources/external_resources.rs"]
mod external_resources;
#[path = "engine_integration/objects/resources/long_form_refs.rs"]
mod long_form_refs;
#[path = "engine_integration/workload/recreate/orphan_sweep.rs"]
mod orphan_sweep;
#[path = "engine_integration/workload/lifecycle/pause_resume.rs"]
mod pause_resume;
#[path = "engine_integration/observe/inspect/port_query.rs"]
mod port_query;
#[path = "engine_integration/workload/recreate/remove_orphans.rs"]
mod remove_orphans;
#[path = "engine_integration/workload/run/run_oneshot.rs"]
mod run_oneshot;
#[path = "engine_integration/workload/scaling/scaled_replicas.rs"]
mod scaled_replicas;
#[path = "engine_integration/observe/inspect/top_images.rs"]
mod top_images;
#[path = "engine_integration/workload/recreate/up_recreate.rs"]
mod up_recreate;

#[cfg(all(unix, feature = "test-helpers"))]
#[path = "engine_integration/host_files/copy/cp_sparse.rs"]
mod cp_sparse;

#[cfg(all(unix, feature = "test-helpers"))]
#[path = "engine_integration/host_files/copy/cp_directory.rs"]
mod cp_directory;

#[path = "engine_integration/objects/network/dns_resolution.rs"]
mod dns_resolution;
#[path = "engine_integration/compose_files/error_surfacing.rs"]
mod error_surfacing;
#[path = "engine_integration/observe/exec/exec_flags.rs"]
mod exec_flags;
#[path = "engine_integration/workload/dependencies/health_waits.rs"]
mod health_waits;
#[path = "engine_integration/compose_files/profiles.rs"]
mod profiles;

#[cfg(feature = "test-helpers")]
#[path = "engine_integration/objects/resources/external_config.rs"]
mod external_config;

#[path = "engine_integration/images/build/build_env_arg.rs"]
mod build_env_arg;
#[path = "engine_integration/compose_files/container_options.rs"]
mod container_options;
#[path = "engine_integration/workload/dependencies/depends_on_health.rs"]
mod depends_on_health;
#[path = "engine_integration/observe/exec/exec_query.rs"]
mod exec_query;
#[path = "engine_integration/workload/dependencies/implicit_dependencies.rs"]
mod implicit_dependencies;
#[path = "engine_integration/compose_files/include_extends.rs"]
mod include_extends;
#[path = "engine_integration/compose_files/label_file_safety.rs"]
mod label_file_safety;
#[path = "engine_integration/workload/lifecycle/lifecycle.rs"]
mod lifecycle;
#[path = "engine_integration/workload/lifecycle/lifecycle_hooks.rs"]
mod lifecycle_hooks;
#[path = "engine_integration/observe/logs/logs_attach.rs"]
mod logs_attach;
#[path = "engine_integration/observe/logs/ps_and_logs.rs"]
mod ps_and_logs;
#[path = "engine_integration/observe/inspect/ps_query.rs"]
mod ps_query;
#[path = "engine_integration/images/registry/pull_query.rs"]
mod pull_query;
#[path = "engine_integration/objects/resources/secrets_and_configs.rs"]
mod secrets_and_configs;
#[path = "engine_integration/workload/scaling/service_selection.rs"]
mod service_selection;
#[path = "engine_integration/workload/scaling/target_services.rs"]
mod target_services;
#[path = "engine_integration/workload/recreate/up_recreate_policy.rs"]
mod up_recreate_policy;

#[cfg(all(unix, feature = "test-helpers"))]
#[path = "engine_integration/libpod_origin/libpod_origin_form_helpers.rs"]
mod libpod_origin_form_helpers;
#[cfg(all(unix, feature = "test-helpers"))]
pub(crate) use libpod_origin_form_helpers::*;
#[cfg(all(unix, feature = "test-helpers"))]
#[path = "engine_integration/libpod_origin/libpod_origin_form_build_comp.rs"]
mod libpod_origin_form_build_comp;
#[cfg(all(unix, feature = "test-helpers"))]
#[path = "engine_integration/libpod_origin/libpod_origin_form_io_comps.rs"]
mod libpod_origin_form_io_comps;

#[cfg(all(unix, feature = "test-helpers"))]
#[path = "engine_integration/libpod_origin/libpod_origin_form_events.rs"]
mod libpod_origin_form_events;

#[cfg(all(unix, feature = "test-helpers"))]
#[path = "engine_integration/libpod_origin/libpod_origin_form_lifecycle_comps.rs"]
mod libpod_origin_form_lifecycle_comps;
#[path = "engine_integration/workload/lifecycle/niche.rs"]
mod niche;
#[path = "engine_integration/workload/recreate/recreate_on_image.rs"]
mod recreate_on_image;
#[path = "engine_integration/workload/run/run_flags.rs"]
mod run_flags;
#[path = "engine_integration/objects/resources/secrets.rs"]
mod secrets;

#[cfg(feature = "test-helpers")]
#[path = "engine_integration/host_files/watch/watch.rs"]
mod watch_tests;

#[cfg(feature = "test-helpers")]
#[path = "engine_integration/host_files/watch/watch_actions.rs"]
mod watch_actions;

#[cfg(feature = "test-helpers")]
#[path = "engine_integration/host_files/watch/watch_sync.rs"]
mod watch_sync;

#[cfg(all(unix, feature = "test-helpers"))]
#[path = "engine_integration/host_files/watch/watch_batch.rs"]
mod watch_batch;

#[cfg(all(unix, feature = "test-helpers"))]
#[path = "engine_integration/host_files/watch/watch_bind.rs"]
mod watch_bind;

#[cfg(all(unix, feature = "test-helpers"))]
#[path = "engine_integration/host_files/watch/watch_sparse.rs"]
mod watch_sparse;

#[cfg(all(unix, feature = "test-helpers"))]
#[path = "engine_integration/host_files/watch/watch_delete.rs"]
mod watch_delete;

#[cfg(all(unix, feature = "test-helpers"))]
#[path = "engine_integration/host_files/watch/watch_ignore_relative.rs"]
mod watch_ignore_relative;

#[path = "engine_integration/workload/autoupdate/x_podman_autoupdate.rs"]
mod x_podman_autoupdate;

#[path = "engine_integration/cli/inspect/cli_absent_project.rs"]
mod cli_absent_project;
#[path = "engine_integration/cli/images_files/cli_build_push.rs"]
mod cli_build_push;
#[path = "engine_integration/cli/lifecycle/cli_commands.rs"]
mod cli_commands;
#[path = "engine_integration/cli/images_files/cli_cp.rs"]
mod cli_cp;
#[path = "engine_integration/cli/lifecycle/cli_down_orphans_rmi.rs"]
mod cli_down_orphans_rmi;
#[path = "engine_integration/cli/logs_exec/cli_exec.rs"]
mod cli_exec;
#[path = "engine_integration/cli/logs_exec/cli_logs.rs"]
mod cli_logs;
#[path = "engine_integration/cli/logs_exec/cli_logs_tail.rs"]
mod cli_logs_tail;
#[path = "engine_integration/cli/inspect/cli_output.rs"]
mod cli_output;
#[path = "engine_integration/cli/inspect/cli_port.rs"]
mod cli_port;
#[path = "engine_integration/cli/inspect/cli_ps.rs"]
mod cli_ps;
#[path = "engine_integration/cli/images_files/cli_pull.rs"]
mod cli_pull;
#[path = "engine_integration/cli/lifecycle/cli_pull_flags.rs"]
mod cli_pull_flags;
#[path = "engine_integration/cli/lifecycle/cli_restart_stop_kill.rs"]
mod cli_restart_stop_kill;
#[path = "engine_integration/cli/lifecycle/cli_run.rs"]
mod cli_run;
#[path = "engine_integration/cli/lifecycle/cli_service_ops.rs"]
mod cli_service_ops;
#[path = "engine_integration/cli/inspect/cli_stats.rs"]
mod cli_stats;
#[path = "engine_integration/cli/inspect/cli_top_images.rs"]
mod cli_top_images;
#[path = "engine_integration/cli/lifecycle/cli_up_down.rs"]
mod cli_up_down;
#[path = "engine_integration/cli/lifecycle/cli_up_flags.rs"]
mod cli_up_flags;
#[path = "engine_integration/cli/inspect/cli_volumes_config.rs"]
mod cli_volumes_config;
#[path = "engine_integration/cli/lifecycle/cli_wait_flags.rs"]
mod cli_wait_flags;
#[path = "engine_integration/workload/lifecycle/create_ls.rs"]
mod create_ls;
#[path = "engine_integration/workload/lifecycle/lifecycle_output.rs"]
mod lifecycle_output;
#[path = "engine_integration/observe/logs/logs_reader_closes.rs"]
mod logs_reader_closes;
#[path = "engine_integration/compose_files/multi_file.rs"]
mod multi_file;
#[path = "engine_integration/objects/network/network_ownership.rs"]
mod network_ownership;
#[path = "engine_integration/objects/network/network_pasta.rs"]
mod network_pasta;
#[path = "engine_integration/workload/pod/pod_create.rs"]
mod pod_create;
#[path = "engine_integration/workload/pod/pod_dns.rs"]
mod pod_dns;
#[path = "engine_integration/workload/pod/pod_exit_policy.rs"]
mod pod_exit_policy;
#[path = "engine_integration/workload/pod/pod_fixtures.rs"]
mod pod_fixtures;
// pasta pods are a Linux feature, and the helpers it uses are Unix-only.
#[cfg(unix)]
#[path = "engine_integration/workload/pod/pod_on_pasta.rs"]
mod pod_on_pasta;
#[path = "engine_integration/workload/pod/pod_recreate.rs"]
mod pod_recreate;
#[path = "engine_integration/workload/pod/pod_teardown.rs"]
mod pod_teardown;
#[path = "engine_integration/workload/pod/pod_userns_mode.rs"]
mod pod_userns_mode;
#[path = "engine_integration/images/registry/push_registry.rs"]
mod push_registry;
#[path = "engine_integration/workload/scaling/scale.rs"]
mod scale;
#[path = "engine_integration/autostart/start_restores_config.rs"]
mod start_restores_config;
#[path = "engine_integration/cli/inspect/stats_flags.rs"]
mod stats_flags;

#[path = "engine_integration/workload/userns/userns.rs"]
mod userns;

#[path = "engine_integration/workload/userns/userns_pod.rs"]
mod userns_pod;

// Unix only: it inspects the replicas through the `/run/user/<uid>` socket.
#[cfg(unix)]
#[path = "engine_integration/workload/userns/keep_id_replicas.rs"]
mod keep_id_replicas;
