//! Positive canned data makes read-only commands prove they still do work.

use std::os::unix::net::UnixStream;

use super::peer::Request;
use super::wire::{ping_ok, reply};

const CONTAINERS: &[u8] = br#"[{"Id":"0123456789abcdef","Names":["hostile-web-1"],"Image":"alpine:3.20","State":"running","Labels":{"podup.project":"hostile","podup.service":"web"}}]"#;
const IMAGE: &[u8] = br#"{"Id":"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","Size":1234,"Created":"2026-01-01T00:00:00Z"}"#;

pub fn answer(stream: &mut UnixStream, request: &Request) {
	if !request.read_only() {
		// Never perform the requested operation; its recorded attempt fails the test.
		reply(
			stream,
			403,
			"",
			br#"{"message":"test peer forbids writes and hijacks"}"#,
		);
		return;
	}
	let path = request.target.split('?').next().unwrap();
	if path.ends_with("/_ping") {
		if request.method == "HEAD" {
			reply(stream, 200, "Libpod-API-Version: 5.0.0\r\n", b"");
		} else {
			ping_ok(stream);
		}
		return;
	}
	let body: &[u8] = if path.ends_with("/containers/json") {
		CONTAINERS
	} else if path.contains("/images/") && path.ends_with("/json") {
		IMAGE
	} else if path.ends_with("/networks/json") || path.ends_with("/pods/json") {
		b"[]"
	} else if path.ends_with("/volumes/json") {
		br#"[{"Name":"hostile_data","Driver":"local","Labels":{"podup.project":"hostile"}}]"#
	} else if path.ends_with("/version") {
		br#"{"Version":"5.0.0","ApiVersion":"5.0.0","Os":"linux","Arch":"amd64"}"#
	} else {
		reply(stream, 404, "", br#"{"message":"uncanned endpoint"}"#);
		return;
	};
	reply(stream, 200, "Content-Type: application/json\r\n", body);
}
