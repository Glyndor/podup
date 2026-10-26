use super::*;

// --- resolve_modifier ---

#[test]
fn resolve_none_uses_value_or_empty() {
	let v = vars(&[("A", "1")]);
	assert_eq!(
		resolve_modifier(
			"A".into(),
			Modifier::None,
			&v,
			0,
			&mut 0usize,
			&mut HashSet::new()
		)
		.unwrap(),
		"1"
	);
	assert_eq!(
		resolve_modifier(
			"MISSING".into(),
			Modifier::None,
			&v,
			0,
			&mut 0usize,
			&mut HashSet::new()
		)
		.unwrap(),
		""
	);
}

#[test]
fn resolve_default_if_unset_or_empty() {
	let v = vars(&[("EMPTY", ""), ("SET", "x")]);
	let m = || Modifier::DefaultIfUnsetOrEmpty("def".into());
	assert_eq!(
		resolve_modifier("EMPTY".into(), m(), &v, 0, &mut 0usize, &mut HashSet::new()).unwrap(),
		"def"
	);
	assert_eq!(
		resolve_modifier(
			"MISSING".into(),
			m(),
			&v,
			0,
			&mut 0usize,
			&mut HashSet::new()
		)
		.unwrap(),
		"def"
	);
	assert_eq!(
		resolve_modifier("SET".into(), m(), &v, 0, &mut 0usize, &mut HashSet::new()).unwrap(),
		"x"
	);
}

#[test]
fn resolve_default_if_unset_keeps_empty_value() {
	let v = vars(&[("EMPTY", "")]);
	assert_eq!(
		resolve_modifier(
			"EMPTY".into(),
			Modifier::DefaultIfUnset("def".into()),
			&v,
			0,
			&mut 0usize,
			&mut HashSet::new()
		)
		.unwrap(),
		""
	);
	assert_eq!(
		resolve_modifier(
			"MISSING".into(),
			Modifier::DefaultIfUnset("def".into()),
			&v,
			0,
			&mut 0usize,
			&mut HashSet::new()
		)
		.unwrap(),
		"def"
	);
}

#[test]
fn resolve_alt_forms() {
	let v = vars(&[("EMPTY", ""), ("SET", "x")]);
	assert_eq!(
		resolve_modifier(
			"SET".into(),
			Modifier::AltIfSetAndNonEmpty("a".into()),
			&v,
			0,
			&mut 0usize,
			&mut HashSet::new()
		)
		.unwrap(),
		"a"
	);
	assert_eq!(
		resolve_modifier(
			"EMPTY".into(),
			Modifier::AltIfSetAndNonEmpty("a".into()),
			&v,
			0,
			&mut 0usize,
			&mut HashSet::new()
		)
		.unwrap(),
		""
	);
	assert_eq!(
		resolve_modifier(
			"EMPTY".into(),
			Modifier::AltIfSet("a".into()),
			&v,
			0,
			&mut 0usize,
			&mut HashSet::new()
		)
		.unwrap(),
		"a"
	);
	assert_eq!(
		resolve_modifier(
			"MISSING".into(),
			Modifier::AltIfSet("a".into()),
			&v,
			0,
			&mut 0usize,
			&mut HashSet::new()
		)
		.unwrap(),
		""
	);
}

#[test]
fn resolve_error_forms() {
	let v = vars(&[("EMPTY", ""), ("SET", "x")]);
	assert!(resolve_modifier(
		"EMPTY".into(),
		Modifier::ErrorIfUnsetOrEmpty("e".into()),
		&v,
		0,
		&mut 0usize,
		&mut HashSet::new()
	)
	.is_err());
	assert_eq!(
		resolve_modifier(
			"SET".into(),
			Modifier::ErrorIfUnsetOrEmpty("e".into()),
			&v,
			0,
			&mut 0usize,
			&mut HashSet::new()
		)
		.unwrap(),
		"x"
	);
	assert!(resolve_modifier(
		"MISSING".into(),
		Modifier::ErrorIfUnset("e".into()),
		&v,
		0,
		&mut 0usize,
		&mut HashSet::new()
	)
	.is_err());
	assert_eq!(
		resolve_modifier(
			"EMPTY".into(),
			Modifier::ErrorIfUnset("e".into()),
			&v,
			0,
			&mut 0usize,
			&mut HashSet::new()
		)
		.unwrap(),
		""
	);
}
