use super::*;
use crate::compose::types::Service;

mod cdi;
mod effective_limits;
mod forwarding;
mod limits;
mod ulimits;

fn default_service() -> Service {
	Service::default()
}
