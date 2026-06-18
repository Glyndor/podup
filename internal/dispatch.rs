//! Command dispatch: map a parsed `Commands` to engine calls.
//!
//! Split out of `main.rs` to keep that file within the source line limit as the
//! CLI surface grows. The match consumes the `Commands` value (arms move their
//! fields); `Config`/`Generate`/`Ls`/`Update`/`Completions` are handled earlier
//! in `main` and reached here only as `unreachable!` guards.

use podup::Engine;

use crate::cli::*;

/// Run the command against an already-built engine and parsed compose file.
pub(crate) async fn dispatch(
	engine: &Engine,
	file: &podup::compose::types::ComposeFile,
	command: Commands,
	profile: &[String],
) -> podup::Result<()> {
	match command {
		Commands::Up {
			detach,
			build,
			watch,
			remove_orphans,
			no_recreate,
			force_recreate,
			no_deps,
			timeout: _,
			scale: _,
			pull: _,
			no_build: _,
			quiet_pull: _,
			wait,
			no_start,
			services,
		} => {
			if remove_orphans {
				engine.remove_orphans(file).await?;
			}
			if build {
				engine.build_all(file, &services).await?;
			}
			// `--no-start` creates the containers but never starts them, so the
			// wait/watch/attach steps below do not apply.
			if no_start {
				engine
					.create_with_options(
						file,
						profile,
						&services,
						no_recreate,
						force_recreate,
						no_deps,
					)
					.await?;
				return Ok(());
			}
			engine
				.up_with_options(
					file,
					detach,
					profile,
					&services,
					no_recreate,
					force_recreate,
					no_deps,
				)
				.await?;
			if wait {
				engine.wait_services_healthy(file, &services).await?;
			}
			if watch {
				engine.watch(file).await?;
			} else if !detach {
				engine.attach_logs(file).await?;
				let _ = engine.stop(file, &[]).await;
			}
		}
		Commands::Down {
			volumes,
			remove_orphans,
			rmi,
			timeout: _,
		} => {
			if remove_orphans {
				engine.remove_orphans(file).await?;
			}
			engine.down_with_options(file, volumes).await?;
			if let Some(scope) = rmi {
				engine
					.remove_service_images(file, scope == RmiScope::Local)
					.await?;
			}
		}
		Commands::Start { services } => engine.start(file, &services).await?,
		Commands::Stop {
			services,
			timeout: _,
		} => engine.stop(file, &services).await?,
		Commands::Scale { pairs } => engine.scale(file, &pairs).await?,
		Commands::Create {
			build,
			force_recreate,
			no_recreate,
			services,
		} => {
			if build {
				engine.build_all(file, &services).await?;
			}
			engine
				.create_with_options(file, profile, &services, no_recreate, force_recreate, false)
				.await?
		}
		Commands::Build {
			no_cache,
			pull,
			build_arg,
			quiet,
			services,
		} => {
			engine
				.build_all_with_options(
					file,
					&services,
					&podup::BuildOptions {
						no_cache,
						pull,
						build_args: build_arg,
						quiet,
					},
				)
				.await?
		}
		Commands::Rm {
			force,
			volumes,
			services,
		} => {
			engine
				.rm_with_options(file, &services, force, volumes)
				.await?
		}
		Commands::Kill {
			signal,
			remove_orphans,
			services,
		} => {
			engine.kill(file, &services, &signal).await?;
			if remove_orphans {
				engine.remove_orphans(file).await?;
			}
		}
		Commands::Pause { services } => engine.pause(file, &services).await?,
		Commands::Unpause { services } => engine.unpause(file, &services).await?,
		Commands::Run {
			service,
			rm,
			detach,
			env_overrides,
			name,
			service_ports,
			user: _,
			workdir: _,
			entrypoint: _,
			volume: _,
			publish: _,
			interactive: _,
			no_tty: _,
			no_deps: _,
			cmd,
		} => {
			engine
				.run(
					file,
					&service,
					podup::RunOptions {
						cmd,
						rm,
						detach,
						env_overrides,
						name_override: name,
						service_ports,
					},
				)
				.await?
		}
		Commands::Cp {
			src,
			dst,
			index,
			follow_link,
			archive,
		} => {
			engine
				.cp_with_options(
					file,
					&src,
					&dst,
					podup::CpOptions {
						index,
						follow_link,
						archive,
					},
				)
				.await?
		}
		Commands::Ps { all, quiet, format } => {
			engine
				.ps_with_options(
					file,
					podup::PsOptions {
						all,
						quiet,
						json: format == OutputFormat::Json,
					},
				)
				.await?
		}
		Commands::Top { services } => engine.top(file, &services).await?,
		Commands::Stats {
			no_stream,
			services,
		} => engine.stats(file, &services, no_stream).await?,
		Commands::Push {
			ignore_push_failures,
			tls_verify,
			services,
		} => {
			engine
				.push(
					file,
					&services,
					podup::PushOptions {
						ignore_failures: ignore_push_failures,
						tls_verify,
					},
				)
				.await?
		}
		Commands::Port {
			service,
			private_port,
			proto,
		} => engine.port(file, &service, private_port, &proto).await?,
		Commands::Images { quiet, format } => {
			engine
				.images_with_options(
					file,
					podup::ImagesOptions {
						quiet,
						json: format == OutputFormat::Json,
					},
				)
				.await?
		}
		Commands::Logs {
			service,
			follow,
			tail,
			since,
			until,
			timestamps,
		} => {
			engine
				.logs_with_options(
					file,
					service.as_deref(),
					podup::LogsOptions {
						follow,
						tail,
						since,
						until,
						timestamps,
					},
				)
				.await?
		}
		Commands::Exec {
			env,
			user,
			workdir,
			privileged,
			detach,
			no_tty: _,
			index,
			service,
			cmd,
		} => {
			engine
				.exec_with_options(
					file,
					&service,
					cmd,
					podup::ExecOptions {
						env,
						user,
						workdir,
						privileged,
						detach,
						index,
					},
				)
				.await?
		}
		Commands::Pull {
			quiet: _,
			ignore_pull_failures,
			include_deps,
			policy: _,
			services,
		} => {
			engine
				.pull_services_with_options(
					file,
					&services,
					podup::PullOptions {
						ignore_failures: ignore_pull_failures,
						include_deps,
					},
				)
				.await?
		}
		Commands::Restart {
			service,
			timeout: _,
			no_deps,
		} => {
			engine
				.restart_with_options(file, service.as_deref(), no_deps)
				.await?
		}
		Commands::Config { .. } => unreachable!("handled above"),
		Commands::Generate { .. } => unreachable!("handled above"),
		Commands::Watch => engine.watch(file).await?,
		Commands::Ls { .. } => unreachable!("handled before compose parsing"),
		#[cfg(feature = "update")]
		Commands::Update { .. } => unreachable!("handled before compose parsing"),
		#[cfg(feature = "completions")]
		Commands::Completions { .. } => unreachable!("handled before compose parsing"),
	}

	Ok(())
}
