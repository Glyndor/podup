use super::{Client, ConnPool};

#[test]
fn an_extreme_pool_size_is_bounded_without_panicking() {
	let client = Client::with_pool_size("unused", usize::MAX);
	assert_eq!(client.pool_size(), 1024);
}

#[test]
fn creating_a_pool_does_not_reserve_all_its_idle_slots() {
	for cap in [0, 1, 1024] {
		let pool = ConnPool::new("unused".into(), cap);
		assert_eq!(pool.cap(), cap);
		assert_eq!(pool.inner.lock().unwrap().idle.capacity(), 0);
	}
}
