use super::*;
use crate::compose::types::Labels;

fn networks() -> HashMap<String, PerNetworkOptions> {
	let options = PerNetworkOptions {
		aliases: vec!["web".into(), "api".into()],
		..Default::default()
	};
	HashMap::from([("p_default".to_string(), options)])
}

fn service(labels: &[&str]) -> Service {
	Service {
		labels: Labels::List(labels.iter().map(|l| l.to_string()).collect()),
		..Default::default()
	}
}

#[test]
fn a_one_off_loses_the_service_name_and_the_declared_aliases() {
	let mut n = networks();
	drop_service_aliases(&service(&["podup.oneoff=true"]), &mut n);
	assert!(n["p_default"].aliases.is_empty());
}

#[test]
fn an_ordinary_service_container_keeps_them() {
	let mut n = networks();
	drop_service_aliases(&service(&["other=1"]), &mut n);
	assert_eq!(n["p_default"].aliases, ["web", "api"]);
	let mut n = networks();
	drop_service_aliases(&service(&["podup.oneoff=false"]), &mut n);
	assert_eq!(n["p_default"].aliases.len(), 2);
}
