//! The long-form volume mount: consistency, and the `volume:` options block
//! with its driver_config, subpath and labels.
//!
//! Split out of `tests/compose/parsing/parse/coverage/network_volume/mod.rs`; the imports are
//! inherited via `use super::*`.

use super::*;

// ---------------------------------------------------------------------------
// Volume: consistency, driver_config, subpath, labels
// ---------------------------------------------------------------------------

#[test]
fn volume_consistency() {
	let yaml = r#"
services:
  app:
    image: alpine
    volumes:
      - type: volume
        source: data
        target: /data
        consistency: cached
"#;
	let v = &parse_str(yaml).unwrap().services["app"].volumes[0];
	match v {
		VolumeMount::Long { consistency, .. } => {
			assert_eq!(consistency.as_deref(), Some("cached"));
		}
		_ => panic!("expected long-form volume"),
	}
}

#[test]
fn volume_options_driver_config() {
	let yaml = r#"
services:
  app:
    image: alpine
    volumes:
      - type: volume
        source: data
        target: /data
        volume:
          nocopy: true
          driver_config:
            name: nfs
            options:
              addr: "nfs.example.com"
"#;
	let v = &parse_str(yaml).unwrap().services["app"].volumes[0];
	match v {
		VolumeMount::Long {
			volume: Some(vo), ..
		} => {
			let dc = vo.driver_config.as_ref().unwrap();
			assert_eq!(dc.name.as_deref(), Some("nfs"));
			assert_eq!(
				dc.options.get("addr").map(|s| s.as_str()),
				Some("nfs.example.com")
			);
		}
		_ => panic!("expected long-form volume with options"),
	}
}

#[test]
fn volume_options_subpath() {
	let yaml = r#"
services:
  app:
    image: alpine
    volumes:
      - type: volume
        source: data
        target: /data
        volume:
          subpath: subdir/nested
"#;
	let v = &parse_str(yaml).unwrap().services["app"].volumes[0];
	match v {
		VolumeMount::Long {
			volume: Some(vo), ..
		} => {
			assert_eq!(vo.subpath.as_deref(), Some("subdir/nested"));
		}
		_ => panic!("expected long-form volume"),
	}
}

#[test]
fn volume_options_labels() {
	let yaml = r#"
services:
  app:
    image: alpine
    volumes:
      - type: volume
        source: data
        target: /data
        volume:
          labels:
            backup: daily
"#;
	let v = &parse_str(yaml).unwrap().services["app"].volumes[0];
	match v {
		VolumeMount::Long {
			volume: Some(vo), ..
		} => {
			let labels = vo.labels.to_map();
			assert_eq!(labels.get("backup").map(|s| s.as_str()), Some("daily"));
		}
		_ => panic!("expected long-form volume"),
	}
}
