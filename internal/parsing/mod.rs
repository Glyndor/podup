//! Reading and interpreting a project: the compose file parser, variable substitution, dotenv and env_file loading, and the scalar value formats (ports, sizes, timestamps).

/// Compose-file parsing, `extends:`/`include:` resolution, and topological
/// service ordering.
pub mod compose;
pub(crate) mod input;
/// Docker Compose `${VAR}`/`$VAR` substitution over raw YAML before parsing.
pub mod substitute;
pub(crate) mod values;
