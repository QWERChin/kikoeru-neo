#!/usr/bin/env bash
# Starts kikoeru-neo on the synthetic library for the iOS Simulator checks,
# behind tls-proxy.py, and scans it. Everything lives under <work dir>:
# the library, the data directory, a throwaway CA (certs/ca.pem, which the
# workflow adds to the simulator) and the logs. ios.env there gives the URLs
# and the listener's credentials to the steps after this one.
# Usage: start-server.sh <kikoeru-neo> <ffmpeg> <ffprobe> <work dir>
# macOS runners ship bash 3.2, so this sticks to what it supports.
set -euo pipefail

usage="usage: start-server.sh <kikoeru-neo> <ffmpeg> <ffprobe> <work dir>"
bin=${1:?$usage}
ffmpeg=${2:?$usage}
ffprobe=${3:?$usage}
work=${4:?$usage}
server_port=${IOS_SERVER_PORT:-8888}
https_port=${IOS_HTTPS_PORT:-8443}
control_port=${IOS_CONTROL_PORT:-8444}
here=$(cd "$(dirname "$0")" && pwd)

mkdir -p "$work/certs" "$work/logs"
mkdir -m 700 "$work/data"
"$here/make-library.sh" "$ffmpeg" "$work/library"

# The CA only has to outlive one run. iOS rejects a leaf without a SAN, an
# EKU for servers, or a validity over 398 days.
certs=$work/certs
cat > "$certs/ca.cnf" <<'CNF'
[req]
distinguished_name = dn
prompt = no
[dn]
CN = kikoeru-neo iOS check CA
[v3_ca]
basicConstraints = critical, CA:TRUE
keyUsage = critical, keyCertSign, cRLSign
subjectKeyIdentifier = hash
CNF
cat > "$certs/leaf.cnf" <<'CNF'
[req]
distinguished_name = dn
prompt = no
[dn]
CN = localhost
CNF
cat > "$certs/leaf.ext" <<'EXT'
basicConstraints = CA:FALSE
keyUsage = critical, digitalSignature, keyEncipherment
extendedKeyUsage = serverAuth
subjectAltName = DNS:localhost, IP:127.0.0.1
authorityKeyIdentifier = keyid
EXT
openssl req -x509 -new -nodes -newkey rsa:2048 -sha256 -days 30 -config "$certs/ca.cnf" \
  -extensions v3_ca -keyout "$certs/ca.key" -out "$certs/ca.pem" 2>/dev/null
openssl req -new -nodes -newkey rsa:2048 -sha256 -config "$certs/leaf.cnf" \
  -keyout "$certs/leaf.key" -out "$certs/leaf.csr" 2>/dev/null
openssl x509 -req -in "$certs/leaf.csr" -CA "$certs/ca.pem" -CAkey "$certs/ca.key" -CAcreateserial \
  -sha256 -days 30 -extfile "$certs/leaf.ext" -out "$certs/leaf.pem" 2>/dev/null

data=$work/data
cat > "$data/config.toml" <<TOML
listen = "127.0.0.1:$server_port"
public_url = "https://localhost:$https_port"

[[roots]]
id = "ios"
display_name = "iOS 合成媒体库"
type = "local"
path = "$work/library"

[media]
ffmpeg = "$ffmpeg"
ffprobe = "$ffprobe"

[metadata]
source = "none"
TOML

admin_password=$(openssl rand -hex 12)
listener_password=$(openssl rand -hex 12)
printf '%s\n' "$admin_password" | "$bin" --data-dir "$data" user create admin --admin --password-stdin
printf '%s\n' "$listener_password" | "$bin" --data-dir "$data" user create listener --password-stdin

nohup "$bin" --data-dir "$data" serve > "$work/logs/server.log" 2>&1 &
echo $! > "$work/server.pid"
nohup python3 "$here/tls-proxy.py" --cert "$certs/leaf.pem" --key "$certs/leaf.key" \
  --listen-port "$https_port" --control-port "$control_port" --upstream-port "$server_port" \
  > "$work/logs/proxy.log" 2>&1 &
echo $! > "$work/proxy.pid"

base=https://localhost:$https_port
api() { curl --silent --show-error --fail --cacert "$certs/ca.pem" "$@"; }
for _ in $(seq 100); do
  api --output /dev/null "$base/api/ready" 2>/dev/null && break
  sleep 0.2
done
api --output /dev/null "$base/api/ready"

login() {
  api -H 'Content-Type: application/json' \
    -d "{\"name\":\"$1\",\"password\":\"$2\"}" "$base/api/auth/login" | jq -r .token
}
auth="Authorization: Bearer $(login admin "$admin_password")"
task=$(api -H "$auth" -H 'Content-Type: application/json' -d '{"root":null}' "$base/api/admin/scan" |
  jq -r '.task_ids[0]')
state=
for _ in $(seq 120); do
  state=$(api -H "$auth" "$base/api/tasks/$task" | jq -r .state)
  case $state in done | failed | cancelled | interrupted) break ;; esac
  sleep 0.5
done
if [ "$state" != done ]; then
  echo "scan task $task ended as '$state'" >&2
  exit 1
fi

# The scan probes durations in the background. The test reads them from
# the track list, so it waits until every track has one.
work_id=$(api -H "$auth" "$base/api/works?page=1" | jq -r '.items[0].id')
tracks=0
for _ in $(seq 120); do
  detail=$(api -H "$auth" "$base/api/works/$work_id")
  tracks=$(echo "$detail" | jq '.tracks | length')
  pending=$(echo "$detail" | jq '[.tracks[] | select(.duration_ms == null)] | length')
  [ "$tracks" -eq 6 ] && [ "$pending" -eq 0 ] && break
  sleep 0.5
done
echo "$detail" | jq -r '.tracks[] | "\(.id)\t\(.duration_ms)\t\(.path)"'
if [ "$tracks" -ne 6 ] || [ "$pending" -ne 0 ]; then
  echo "expected 6 probed tracks, got $tracks with $pending unprobed" >&2
  exit 1
fi

cat > "$work/ios.env" <<ENV
IOS_BASE_URL=$base
IOS_CONTROL_URL=https://localhost:$control_port
IOS_USER=listener
IOS_PASSWORD=$listener_password
IOS_WORK_ID=$work_id
ENV
echo "kikoeru-neo is up at $base with work $work_id"
