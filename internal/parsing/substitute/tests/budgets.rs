use super::*;

#[test]
fn deeply_nested_defaults_error_instead_of_overflowing() {
	// A pathological `${A:-${A:-…}}` chain (all default branches taken, A unset)
	// returns a clean error past the depth cap rather than overflowing the stack.
	let depth = MAX_INTERP_DEPTH + 50;
	let mut s = String::new();
	for _ in 0..depth {
		s.push_str("${A:-");
	}
	s.push('x');
	for _ in 0..depth {
		s.push('}');
	}
	let err = substitute(&s, &vars(&[])).expect_err("over-deep nesting must error");
	assert!(matches!(
		err,
		crate::error::ComposeError::InvalidSubstitution(_)
	));
}

#[test]
fn moderate_nesting_still_resolves() {
	// Well within the cap, nested defaults resolve normally.
	assert_eq!(
		substitute("${A:-${B:-${C:-deep}}}", &vars(&[])).unwrap(),
		"deep"
	);
}

// variable-repetition cap (#1738)

#[test]
fn interpolation_cap_refuses_variable_repetition_in_one_scalar() {
	// 300 KB of `${PAYLOAD}` references in one scalar with a moderately sized
	// PAYLOAD value would otherwise produce hundreds of MB of output and
	// (on the host from the issue) SIGABRT in under a second. The cap is
	// checked as `out` grows inside `substitute_depth`, so the refusal
	// fires before the offending allocation is committed, and names both
	// the variable and the size so a legitimate large value can be told
	// apart from a hostile one (#1738).
	let payload = "x".repeat(2 * 1024);
	let mut input = String::new();
	while input.len() < 300 * 1024 {
		input.push_str("${PAYLOAD}");
	}
	let v = vars(&[("PAYLOAD", &payload)]);
	let err = substitute(&input, &v)
		.expect_err("expected the interpolation cap to refuse variable repetition");
	let msg = format!("{err}");
	assert!(
		matches!(err, crate::error::ComposeError::InvalidSubstitution(_)),
		"refusal should be InvalidSubstitution; got: {msg}"
	);
	assert!(
		msg.contains("PAYLOAD"),
		"refusal should name the variable; got: {msg}"
	);
	assert!(
		msg.contains("bytes"),
		"refusal should name the size reached; got: {msg}"
	);
}

#[test]
fn interpolation_cap_allows_legitimate_repetition() {
	// A handful of repetitions of a small value stay well under the cap.
	let v = vars(&[("TAG", "v1")]);
	assert_eq!(substitute("${TAG}-${TAG}-${TAG}", &v).unwrap(), "v1-v1-v1");
}

#[test]
fn the_document_budget_bounds_repetition_spread_across_scalars() {
	// The per-scalar cap is the wrong unit on its own: repetition split
	// across scalars is unbounded while every single substitution stays
	// legal. Measured before this budget existed, with a 1 MiB value and
	// one service reading it from N environment entries: 256 reached
	// 1.6 GB, 512 reached 3.2 GB, 1024 aborted at 5.27 GB, and no single
	// substitution passed 1 MiB, so the per-scalar cap never fired.
	//
	// The value is built here rather than committed as a fixture.
	let big = "y".repeat(1024 * 1024);
	let mut vars = std::collections::HashMap::new();
	vars.insert("BIG".to_string(), big);

	// Under the budget: still accepted, because a long secret across a
	// handful of fields is a legitimate document.
	let mut spent = 0usize;
	let mut warned = std::collections::HashSet::new();
	for _ in 0..8 {
		super::substitute_budgeted("${BIG}", &vars, &mut spent, &mut warned)
			.expect("eight one-mebibyte substitutions stay under the budget");
	}

	// Over it: refused, and the message names the variable so a large
	// legitimate file can be told apart from a hostile one.
	let mut spent = 0usize;
	let mut warned = std::collections::HashSet::new();
	let err = (0..64)
		.map(|_| super::substitute_budgeted("${BIG}", &vars, &mut spent, &mut warned))
		.find_map(|r| r.err())
		.expect("sixty-four must cross the document budget");
	let msg = format!("{err}");
	assert!(
		msg.contains("BIG") && msg.contains("past"),
		"the refusal must name the variable and the budget, got: {msg}"
	);
}
