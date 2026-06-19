//! Parse coverage for compose-spec keys that previously fell into the
//! `unknown`/`extensions` bucket: `credential_spec`, `isolation`, `provider`,
//! `use_api_socket` (service-level) and the top-level `models` element. Each
//! must deserialize into its dedicated field and no longer trip the generic
//! "unknown key" diagnostic; the not-honored diagnostic is asserted in
//! `tests/cli_diagnostics.rs` and the diagnostics unit tests.
use podup::parse_str;

#[test]
fn credential_spec_object_parses_into_field() {
	let yaml = r#"
services:
  web:
    image: nginx
    credential_spec:
      config: my-credential-spec
"#;
	let svc = &parse_str(yaml).unwrap().services["web"];
	let cred = svc
		.credential_spec
		.as_ref()
		.expect("credential_spec parsed");
	assert_eq!(cred.config.as_deref(), Some("my-credential-spec"));
	assert!(cred.file.is_none());
	assert!(cred.registry.is_none());
	// Must not leak into the unknown-key bucket.
	assert!(!svc.unknown.contains_key("credential_spec"));
}

#[test]
fn credential_spec_file_and_registry_parse() {
	let yaml = r#"
services:
  a:
    image: nginx
    credential_spec:
      file: spec.json
  b:
    image: nginx
    credential_spec:
      registry: my-registry-value
"#;
	let file = parse_str(yaml).unwrap();
	assert_eq!(
		file.services["a"]
			.credential_spec
			.as_ref()
			.unwrap()
			.file
			.as_deref(),
		Some("spec.json")
	);
	assert_eq!(
		file.services["b"]
			.credential_spec
			.as_ref()
			.unwrap()
			.registry
			.as_deref(),
		Some("my-registry-value")
	);
}

#[test]
fn service_isolation_parses_into_field() {
	let yaml = "services:\n  web:\n    image: nginx\n    isolation: hyperv\n";
	let svc = &parse_str(yaml).unwrap().services["web"];
	assert_eq!(svc.isolation.as_deref(), Some("hyperv"));
	assert!(!svc.unknown.contains_key("isolation"));
}

#[test]
fn provider_object_parses_into_field() {
	let yaml = r#"
services:
  db:
    provider:
      type: awesomecloud
      options:
        type: mysql
        size: 256
"#;
	let svc = &parse_str(yaml).unwrap().services["db"];
	let provider = svc.provider.as_ref().expect("provider parsed");
	assert_eq!(provider.provider_type.as_deref(), Some("awesomecloud"));
	assert_eq!(
		provider.options.get("type").and_then(|v| v.as_str()),
		Some("mysql")
	);
	assert!(!svc.unknown.contains_key("provider"));
}

#[test]
fn use_api_socket_parses_into_field() {
	let yaml = "services:\n  web:\n    image: nginx\n    use_api_socket: true\n";
	let svc = &parse_str(yaml).unwrap().services["web"];
	assert_eq!(svc.use_api_socket, Some(true));
	assert!(!svc.unknown.contains_key("use_api_socket"));
}

#[test]
fn top_level_models_parses_into_field() {
	let yaml = r#"
services:
  web:
    image: nginx
models:
  llm:
    model: ai/model
    context_size: 1024
    runtime_flags:
      - "--a-flag"
      - "--another-flag=42"
"#;
	let file = parse_str(yaml).unwrap();
	let model = file.models.get("llm").expect("model parsed");
	assert_eq!(model.model.as_deref(), Some("ai/model"));
	assert_eq!(model.context_size, Some(1024));
	assert_eq!(model.runtime_flags.len(), 2);
	// Top-level models must not land in the extensions bucket.
	assert!(!file.extensions.contains_key("models"));
}

#[test]
fn new_keys_produce_no_unknown_key_diagnostic() {
	// All five keys together: none should be reported as an unknown/typo key
	// (the not-honored warnings are a separate, expected signal).
	let yaml = r#"
services:
  web:
    image: nginx
    isolation: default
    use_api_socket: true
    credential_spec:
      file: spec.json
    provider:
      type: cloud
models:
  llm:
    model: ai/model
"#;
	let file = parse_str(yaml).unwrap();
	let svc = &file.services["web"];
	for key in ["isolation", "use_api_socket", "credential_spec", "provider"] {
		assert!(
			!svc.unknown.contains_key(key),
			"service key '{key}' leaked into unknown bucket"
		);
	}
	assert!(!file.extensions.contains_key("models"));
}
