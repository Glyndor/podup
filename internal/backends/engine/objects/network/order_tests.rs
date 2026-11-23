use super::*;

fn attached(entries: &[(&str, Option<u32>)]) -> HashMap<String, PerNetworkOptions> {
	entries
		.iter()
		.map(|(name, priority)| {
			(
				(*name).to_string(),
				PerNetworkOptions {
					priority: *priority,
					..Default::default()
				},
			)
		})
		.collect()
}

#[test]
fn no_priority_asks_for_no_order() {
	assert_eq!(network_order(&attached(&[("a", None), ("b", None)])), None);
	assert_eq!(
		network_order(&attached(&[("a", Some(0)), ("b", None)])),
		None,
		"0 is the default, so it asks for nothing"
	);
	assert_eq!(network_order(&HashMap::new()), None);
}

#[test]
fn the_highest_priority_comes_first() {
	let order = network_order(&attached(&[
		("a", Some(10)),
		("b", Some(1000)),
		("c", Some(100)),
	]));
	assert_eq!(order, Some(vec!["b".into(), "c".into(), "a".into()]));
}

#[test]
fn equal_priorities_keep_the_order_by_name() {
	let order = network_order(&attached(&[
		("z", Some(5)),
		("m", None),
		("a", Some(5)),
		("k", Some(0)),
	]));
	assert_eq!(
		order,
		Some(vec!["a".into(), "z".into(), "k".into(), "m".into()])
	);
}

#[test]
fn a_negative_default_is_not_possible_so_only_larger_values_move_a_network() {
	// `priority` is unsigned; a network with none sits at 0, below any set one.
	let order = network_order(&attached(&[("a", None), ("b", Some(1))]));
	assert_eq!(order, Some(vec!["b".into(), "a".into()]));
}

#[test]
fn by_priority_orders_names_without_losing_or_repeating_any() {
	let priority = |name: &str| match name {
		"a" => 3,
		"b" => 3,
		"c" => 9,
		_ => 0,
	};
	let names: Vec<String> = ["z", "b", "c", "a", "y"]
		.iter()
		.map(|s| s.to_string())
		.collect();
	assert_eq!(by_priority(names, priority), vec!["c", "a", "b", "y", "z"]);
	assert!(by_priority(Vec::new(), priority).is_empty());
}
