use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};

/// Compute a topological start order for all services (Kahn's algorithm).
///
/// Returns service names dependencies-first.
/// Errors on cycles ([`ComposeError::CircularDependency`]) or missing required
/// dependencies ([`ComposeError::ServiceNotFound`]).
pub fn resolve_order(file: &ComposeFile) -> Result<Vec<String>> {
	let services: Vec<&str> = file.services.keys().map(|s| s.as_str()).collect();
	let mut in_degree: HashMap<&str, usize> = services.iter().map(|&s| (s, 0)).collect();
	let mut graph: HashMap<&str, Vec<&str>> = services.iter().map(|&s| (s, vec![])).collect();

	for (name, service) in &file.services {
		for dep in service.depends_on.service_names() {
			if !file.services.contains_key(&dep) {
				if !service.depends_on.required_for(&dep) {
					continue;
				}
				return Err(ComposeError::ServiceNotFound(dep));
			}
			if let Some(neighbors) = graph.get_mut(dep.as_str()) {
				neighbors.push(name.as_str());
			}
			if let Some(deg) = in_degree.get_mut(name.as_str()) {
				*deg += 1;
			}
		}
	}

	// A min-heap (lexicographically smallest name first) keeps the order
	// deterministic: the in-degree map is a `HashMap`, so seeding/extending the
	// frontier from its iteration order would otherwise be per-run random. This
	// mirrors the per-level `sort_unstable` in `resolve_levels`, so independent
	// (in-degree-0) services resolve in a stable order — which `wait` relies on
	// for a reproducible exit code and output ordering.
	let mut queue: BinaryHeap<Reverse<&str>> = in_degree
		.iter()
		.filter(|(_, &deg)| deg == 0)
		.map(|(&s, _)| Reverse(s))
		.collect();

	let mut order = Vec::new();
	while let Some(Reverse(node)) = queue.pop() {
		order.push(node.to_string());
		let neighbors: Vec<&str> = graph.get(node).map_or(&[][..], |v| v.as_slice()).to_vec();
		for neighbor in neighbors {
			if let Some(deg) = in_degree.get_mut(neighbor) {
				*deg -= 1;
				if *deg == 0 {
					queue.push(Reverse(neighbor));
				}
			}
		}
	}

	if order.len() != services.len() {
		return Err(ComposeError::CircularDependency(
			"cycle detected in depends_on".into(),
		));
	}

	Ok(order)
}

/// Group services into dependency levels (Kahn's algorithm, layered).
///
/// Each returned level contains services whose dependencies all live in earlier
/// levels, so the services within one level have no `depends_on` relationship to
/// each other and can be started concurrently. Levels are ordered
/// dependencies-first; names within a level are sorted for deterministic
/// dispatch. Errors on cycles or missing required dependencies, matching
/// [`resolve_order`].
pub fn resolve_levels(file: &ComposeFile) -> Result<Vec<Vec<String>>> {
	let services: Vec<&str> = file.services.keys().map(|s| s.as_str()).collect();
	let mut in_degree: HashMap<&str, usize> = services.iter().map(|&s| (s, 0)).collect();
	let mut graph: HashMap<&str, Vec<&str>> = services.iter().map(|&s| (s, vec![])).collect();

	for (name, service) in &file.services {
		for dep in service.depends_on.service_names() {
			if !file.services.contains_key(&dep) {
				if !service.depends_on.required_for(&dep) {
					continue;
				}
				return Err(ComposeError::ServiceNotFound(dep));
			}
			if let Some(neighbors) = graph.get_mut(dep.as_str()) {
				neighbors.push(name.as_str());
			}
			if let Some(deg) = in_degree.get_mut(name.as_str()) {
				*deg += 1;
			}
		}
	}

	let mut current: Vec<&str> = in_degree
		.iter()
		.filter(|(_, &deg)| deg == 0)
		.map(|(&s, _)| s)
		.collect();

	let mut levels: Vec<Vec<String>> = Vec::new();
	let mut processed = 0;
	while !current.is_empty() {
		current.sort_unstable();
		let mut next: Vec<&str> = Vec::new();
		for &node in &current {
			processed += 1;
			let neighbors: Vec<&str> = graph.get(node).map_or(&[][..], |v| v.as_slice()).to_vec();
			for neighbor in neighbors {
				if let Some(deg) = in_degree.get_mut(neighbor) {
					*deg -= 1;
					if *deg == 0 {
						next.push(neighbor);
					}
				}
			}
		}
		levels.push(current.iter().map(|s| s.to_string()).collect());
		current = next;
	}

	if processed != services.len() {
		return Err(ComposeError::CircularDependency(
			"cycle detected in depends_on".into(),
		));
	}

	Ok(levels)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::parse_str_raw;

	// resolve_order

	#[test]
	fn resolve_order_no_deps_arbitrary_order() {
		let yaml = "services:\n  a:\n    image: x\n  b:\n    image: y\n";
		let file = parse_str_raw(yaml).unwrap();
		let order = resolve_order(&file).unwrap();
		assert_eq!(order.len(), 2);
		assert!(order.contains(&"a".to_string()));
		assert!(order.contains(&"b".to_string()));
	}

	#[test]
	fn resolve_order_is_deterministic_for_independent_services() {
		// Independent (in-degree-0) services must resolve in a stable,
		// lexicographic order regardless of the HashMap iteration order, so
		// `wait`'s exit code and printed order are reproducible across runs.
		let yaml = "services:\n  c:\n    image: x\n  a:\n    image: y\n  b:\n    image: z\n";
		let file = parse_str_raw(yaml).unwrap();
		let order = resolve_order(&file).unwrap();
		assert_eq!(
			order,
			vec!["a".to_string(), "b".to_string(), "c".to_string()]
		);
		// Re-resolving yields the identical order.
		for _ in 0..16 {
			assert_eq!(resolve_order(&file).unwrap(), order);
		}
	}

	#[test]
	fn resolve_order_dependents_are_deterministic() {
		// Two dependents of the same dependency come out in stable lexicographic
		// order, not whatever the graph's adjacency iteration happens to be.
		let yaml = "services:\n  db:\n    image: x\n  zeb:\n    image: y\n    depends_on: [db]\n  api:\n    image: z\n    depends_on: [db]\n";
		let file = parse_str_raw(yaml).unwrap();
		let order = resolve_order(&file).unwrap();
		assert_eq!(
			order,
			vec!["db".to_string(), "api".to_string(), "zeb".to_string()]
		);
	}

	#[test]
	fn resolve_order_dep_before_dependent() {
		let yaml = "services:\n  web:\n    image: nginx\n    depends_on: [db]\n  db:\n    image: postgres\n";
		let file = parse_str_raw(yaml).unwrap();
		let order = resolve_order(&file).unwrap();
		let db_pos = order.iter().position(|s| s == "db").unwrap();
		let web_pos = order.iter().position(|s| s == "web").unwrap();
		assert!(db_pos < web_pos, "db must start before web");
	}

	#[test]
	fn resolve_order_is_deterministic_for_independent_services() {
		// Independent services must resolve in a stable (compose-file) order on
		// every call, so best-effort consumers like `kill` behave reproducibly
		// rather than depending on HashMap iteration order.
		let yaml = "services:\n  a:\n    image: x\n  b:\n    image: y\n  c:\n    image: z\n";
		let file = parse_str_raw(yaml).unwrap();
		let first = resolve_order(&file).unwrap();
		assert_eq!(
			first,
			vec!["a".to_string(), "b".to_string(), "c".to_string()]
		);
		for _ in 0..16 {
			assert_eq!(resolve_order(&file).unwrap(), first);
		}
	}

	#[test]
	fn resolve_order_cycle_is_error() {
		let yaml = "services:\n  a:\n    image: x\n    depends_on: [b]\n  b:\n    image: y\n    depends_on: [a]\n";
		let file = parse_str_raw(yaml).unwrap();
		assert!(resolve_order(&file).is_err());
	}

	#[test]
	fn resolve_order_missing_required_dep_is_error() {
		let yaml = "services:\n  web:\n    image: nginx\n    depends_on: [db]\n";
		let file = parse_str_raw(yaml).unwrap();
		assert!(resolve_order(&file).is_err());
	}

	// resolve_levels

	#[test]
	fn resolve_levels_groups_independent_services_together() {
		let yaml = "services:\n  a:\n    image: x\n  b:\n    image: y\n";
		let file = parse_str_raw(yaml).unwrap();
		let levels = resolve_levels(&file).unwrap();
		// No deps → one level holding both, sorted for determinism.
		assert_eq!(levels, vec![vec!["a".to_string(), "b".to_string()]]);
	}

	#[test]
	fn resolve_levels_orders_dependencies_into_earlier_levels() {
		let yaml = "services:\n  web:\n    image: nginx\n    depends_on: [db]\n  db:\n    image: postgres\n  cache:\n    image: redis\n";
		let file = parse_str_raw(yaml).unwrap();
		let levels = resolve_levels(&file).unwrap();
		// Level 0: db + cache (no deps); level 1: web (depends on db).
		assert_eq!(levels[0], vec!["cache".to_string(), "db".to_string()]);
		assert_eq!(levels[1], vec!["web".to_string()]);
	}

	#[test]
	fn resolve_levels_cycle_is_error() {
		let yaml = "services:\n  a:\n    image: x\n    depends_on: [b]\n  b:\n    image: y\n    depends_on: [a]\n";
		let file = parse_str_raw(yaml).unwrap();
		assert!(resolve_levels(&file).is_err());
	}

	#[test]
	fn resolve_levels_missing_required_dep_is_error() {
		let yaml = "services:\n  web:\n    image: nginx\n    depends_on: [db]\n";
		let file = parse_str_raw(yaml).unwrap();
		assert!(resolve_levels(&file).is_err());
	}

	#[test]
	fn resolve_order_optional_missing_dep_is_ignored() {
		// A `required: false` dependency that is not defined is skipped, not an
		// error — the dependent still resolves.
		let yaml = "services:\n  web:\n    image: nginx\n    depends_on:\n      ghost:\n        condition: service_started\n        required: false\n";
		let file = parse_str_raw(yaml).unwrap();
		let order = resolve_order(&file).unwrap();
		assert_eq!(order, vec!["web".to_string()]);
	}

	#[test]
	fn resolve_levels_optional_missing_dep_is_ignored() {
		let yaml = "services:\n  web:\n    image: nginx\n    depends_on:\n      ghost:\n        condition: service_started\n        required: false\n";
		let file = parse_str_raw(yaml).unwrap();
		let levels = resolve_levels(&file).unwrap();
		assert_eq!(levels, vec![vec!["web".to_string()]]);
	}
}
