#!/usr/bin/env bash
# The installer's download step against an origin that misbehaves: a body
# larger than the 200 MB ceiling, and a body cut short. Both must stop the
# install at the download, before anything is verified or staged, and the
# podup that was already installed must survive untouched.
#
# The other install tests redefine `download`, and proto-redir-test.sh only
# drives the redirect, so the real curl call never met either case. Here the
# production `download` and `install_binary` run against a local https origin
# (python3 + a throwaway openssl certificate, 127.0.0.1 only). The signature
# check is the one stub, as in install-no-downgrade.test.sh; the checksum
# gate is the production function.
#
# The oversize case declares a body one byte over the ceiling and sends a
# few bytes. curl refuses on the header alone, so no 200 MB crosses the
# loopback, and the refusal is attributed to curl's own "Maximum file size
# exceeded": without that, a broken ceiling would still fail the case,
# because the declared length is never delivered.
#
# A control serves a complete release from the same origin and must install,
# so a harness that always failed would not satisfy the refusals.
# Production helpers are loaded dynamically, so their consumers are not visible here.
# shellcheck disable=SC2034,SC2317,SC2329
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
INSTALL_SH="$HERE/install.sh"

pass=0
fail=0
check() {
	if [[ "$2" == "$3" ]]; then
		echo "ok    $1"
		pass=$((pass + 1))
	else
		echo "FAIL  $1 (expected '$2', got '$3')"
		fail=$((fail + 1))
	fi
}
finish() {
	printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
	[[ "$fail" -eq 0 ]]
}

# A missing tool is a missing fixture; the CI runner has both.
for tool in python3 openssl curl; do
	if ! command -v "$tool" >/dev/null 2>&1; then
		echo "SKIP: $tool is not installed"
		finish
		exit
	fi
done

WORK="$(mktemp -d)"
SRV_PID=""
cleanup() {
	if [[ -n "$SRV_PID" ]]; then
		kill "$SRV_PID" 2>/dev/null || true
		wait "$SRV_PID" 2>/dev/null || true
	fi
	rm -rf "$WORK"
}
trap cleanup EXIT

HELPERS="$WORK/helpers.sh"
for name in download verify_checksum install_binary installed_mode verify_no_downgrade compare_versions verify_version_self_test run_root; do
	awk "/^${name}\\(\\) {/,/^}$/" "$INSTALL_SH" >> "$HELPERS"
done
# shellcheck disable=SC1090
source "$HELPERS"

# The complete release the origin serves under /ok/.
ARTIFACT_NAME=podup-linux-x86_64
mkdir -p "$WORK/root/ok"
printf '#!/bin/sh\necho "podup 1.2.3"\n' > "$WORK/root/ok/$ARTIFACT_NAME"
printf '%s  %s\n' "$(sha256sum < "$WORK/root/ok/$ARTIFACT_NAME" | cut -d' ' -f1)" "$ARTIFACT_NAME" > "$WORK/root/ok/SHA256SUMS"
printf 'signature-bytes\n' > "$WORK/root/ok/SHA256SUMS.sig"

PORT="$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')"
openssl req -x509 -newkey rsa:2048 -keyout "$WORK/cert.key" -out "$WORK/cert.pem" \
	-days 1 -nodes -subj "/CN=localhost" -addext "subjectAltName = IP:127.0.0.1" >/dev/null 2>&1

env WORK="$WORK" PORT="$PORT" python3 -I - >"$WORK/srv.log" 2>&1 <<'PYEOF' &
import http.server, os, signal, ssl, threading

WORK = os.environ['WORK']
ROOT = os.path.join(WORK, 'root', 'ok')
CEILING = 209715200
NAMES = {'podup-linux-x86_64', 'SHA256SUMS', 'SHA256SUMS.sig'}


class Handler(http.server.BaseHTTPRequestHandler):
	def do_GET(self):
		try:
			mode, _, name = self.path.lstrip('/').partition('/')
			if name not in NAMES:
				self.send_error(404)
				return
			with open(os.path.join(ROOT, name), 'rb') as f:
				data = f.read()
			declared, body = len(data), data
			if mode == 'oversize':
				declared = CEILING + 1
			elif mode == 'truncated' or (mode == 'truncated-sums' and name == 'SHA256SUMS'):
				body = data[: len(data) // 2]
			self.send_response(200)
			self.send_header('Content-Type', 'application/octet-stream')
			self.send_header('Content-Length', str(declared))
			self.end_headers()
			self.wfile.write(body)
		except (BrokenPipeError, ConnectionResetError, ssl.SSLError):
			pass

	def log_message(self, *a, **k):
		pass


ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.load_cert_chain(os.path.join(WORK, 'cert.pem'), os.path.join(WORK, 'cert.key'))
httpd = http.server.ThreadingHTTPServer(('127.0.0.1', int(os.environ['PORT'])), Handler)
httpd.socket = ctx.wrap_socket(httpd.socket, server_side=True)
threading.Thread(target=httpd.serve_forever, daemon=True).start()

stop = threading.Event()
signal.signal(signal.SIGTERM, lambda *_: stop.set())
signal.signal(signal.SIGINT, lambda *_: stop.set())
open(os.path.join(WORK, 'srv.ready'), 'w').write('ok')
stop.wait()
PYEOF
SRV_PID=$!

for _ in $(seq 1 50); do
	[[ -f "$WORK/srv.ready" ]] && break
	sleep 0.1
done
if [[ ! -f "$WORK/srv.ready" ]]; then
	echo "FAIL: the local origin never came up"
	sed 's/^/        /' "$WORK/srv.log"
	exit 1
fi
export CURL_CA_BUNDLE="$WORK/cert.pem"

# flow <mode> <exit> <pattern the output must match>
flow() {
	local mode="$1" expected="$2" pattern="$3"
	local dir="$WORK/run-$mode" rc=0 before after
	mkdir -p "$dir/bin" "$dir/tmp"
	printf '#!/bin/sh\necho "podup 1.0.0"\n' > "$dir/bin/podup"
	chmod 755 "$dir/bin/podup"
	before="$(cksum < "$dir/bin/podup") $(stat -c %i "$dir/bin/podup")"
	(
		set -euo pipefail
		INSTALL_DIR="$dir/bin"
		TMP_DIR="$dir/tmp"
		VERSION=v1.2.3
		REPO=Glyndor/podup
		ARTIFACT="$ARTIFACT_NAME"
		BASE_URL="https://127.0.0.1:$PORT/$mode"
		SHA256_CMD=(sha256sum)
		log_info() { :; }
		log_ok() { printf '%s\n' "$*"; }
		log_error() { printf '%s\n' "$*" >&2; }
		fail() { log_error "$*"; exit 1; }
		command() {
			if [[ "$*" == '-v gh' ]]; then return 1; fi
			builtin command "$@"
		}
		ed25519_verify() { return 0; }
		install_binary
	) > "$dir/output" 2>&1 || rc=$?
	check "$mode exit status" "$expected" "$rc"
	check "$mode output" 1 "$(grep -Eq "$pattern" "$dir/output" && echo 1 || echo 0)"
	if [[ "$expected" == 1 ]]; then
		after="$(cksum < "$dir/bin/podup") $(stat -c %i "$dir/bin/podup")"
		check "$mode preserves bytes and inode" "$before" "$after"
		check "$mode leaves no staging" 0 "$(find "$dir/bin" -name '.podup.install-*' | wc -l | tr -d ' ')"
		check "$mode stops at the download" 1 "$(grep -q 'Download failed' "$dir/output" && echo 1 || echo 0)"
	else
		check "$mode installs the release" "$(cksum < "$WORK/root/ok/$ARTIFACT_NAME")" "$(cksum < "$dir/bin/podup")"
	fi
}

flow ok 0 'podup 1.2.3'
flow oversize 1 'Maximum file size exceeded'
flow truncated 1 'curl: \((18|56)\)'
flow truncated-sums 1 'curl: \((18|56)\)'

finish
