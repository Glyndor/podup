//! Complex volumes: the long-form bind, tmpfs and volume mounts.
//!
//! Split out of `tests/compose/parsing/parse/schema/fields/extends_build/mod.rs`; the imports are
//! inherited via `use super::*`.

use super::*;

#[test]
fn volume_long_form_bind_propagation() {
	let yaml = r#"
services:
  app:
    image: alpine
    volumes:
      - type: bind
        source: /host
        target: /cont
        bind:
          propagation: rshared
          create_host_path: true
          selinux: z
"#;
	let file = parse_str(yaml).unwrap();
	let v = &file.services["app"].volumes[0];
	match v {
		VolumeMount::Long { bind: Some(b), .. } => {
			assert_eq!(b.propagation.as_deref(), Some("rshared"));
			assert_eq!(b.create_host_path, Some(true));
			assert_eq!(b.selinux.as_deref(), Some("z"));
		}
		_ => panic!("expected long-form bind mount"),
	}
}

#[test]
fn volume_long_form_tmpfs_size_mode() {
	let yaml = r#"
services:
  app:
    image: alpine
    volumes:
      - type: tmpfs
        target: /run
        tmpfs:
          size: 67108864
          mode: 1750
"#;
	let v = &parse_str(yaml).unwrap().services["app"].volumes[0];
	match v {
		VolumeMount::Long { tmpfs: Some(t), .. } => {
			assert_eq!(t.size, Some(67108864));
			// A bare `mode:` is octal (issue #917): `1750` is `0o1750`, the same
			// bits `0o1750`/`01750` would yield, not the decimal 1750.
			assert_eq!(t.mode, Some(0o1750));
		}
		_ => panic!("expected tmpfs mount"),
	}
}

#[test]
fn volume_long_form_volume_nocopy() {
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
"#;
	let v = &parse_str(yaml).unwrap().services["app"].volumes[0];
	match v {
		VolumeMount::Long {
			volume: Some(vo), ..
		} => assert_eq!(vo.nocopy, Some(true)),
		_ => panic!("expected long-form volume mount"),
	}
}
