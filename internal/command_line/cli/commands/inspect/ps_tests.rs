use crate::cli::{Cli, Commands, PsFormat};
use clap::Parser;

#[test]
fn ps_output_flags_conflict_and_ndjson_is_exclusive_to_ps() {
	for flags in [
		vec!["--all-services", "--quiet"],
		vec!["--all-services", "--services"],
		vec!["--all-services", "--format", "table"],
		vec!["--all-services", "--format", "json"],
		vec!["--all-services", "--format", "ndjson"],
		vec!["--services", "--format", "ndjson"],
		vec!["--quiet", "--format", "ndjson"],
	] {
		let mut args = vec!["podup", "ps"];
		args.extend(flags);
		let err = Cli::try_parse_from(args).err().expect("conflict");
		assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
	}
	let cli = Cli::try_parse_from(["podup", "ps", "--format", "ndjson"]).unwrap();
	assert!(matches!(cli.command, Commands::Ps(args) if args.format == PsFormat::Ndjson));
	let cli = Cli::try_parse_from(["podup", "ps", "--all-services", "web"]).unwrap();
	assert!(
		matches!(cli.command, Commands::Ps(args) if args.all_services && args.services == ["web"])
	);
	for cmd in ["ls", "images", "top", "stats"] {
		assert!(Cli::try_parse_from(["podup", cmd, "--format", "ndjson"]).is_err());
	}
}
