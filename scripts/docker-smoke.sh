#!/usr/bin/env bash
# Only disposable named volumes and a synthetic administrator are used.
set -euo pipefail
image=${1:?usage: docker-smoke.sh IMAGE}
tmp=$(mktemp -d)
name="neo-smoke-${tmp##*.}"
cleanup() {
  docker logs "$name" 2>&1 || true
  docker rm -fv "$name" >/dev/null 2>&1 || true
  docker volume rm "$name-data" "$name-backup" >/dev/null 2>&1 || true
  rm -rf "$tmp"
}
trap cleanup EXIT
printf 'listen = "0.0.0.0:8888"\n' > "$tmp/config.toml"
chmod 755 "$tmp"
chmod 644 "$tmp/config.toml"
docker run -d --name "$name" --read-only --tmpfs /tmp \
  -p 127.0.0.1::8888 \
  -v "$name-data:/data" -v "$name-backup:/backup" \
  -v "$tmp/config.toml:/config/config.toml:ro" "$image"
# Management commands also run migrations, so wait for first-start
# migrations to finish before opening the database in another process.
for _ in $(seq 1 90); do
  health=$(docker inspect -f '{{.State.Health.Status}}' "$name")
  [ "$health" != healthy ] || break
  sleep 2
done
[ "$health" = healthy ]
port=$(docker port "$name" 8888/tcp)
test "$(curl -sS -o /dev/null -w '%{http_code}' "http://$port/api/ready")" = 200
printf 'synthetic-admin-password\n' | docker exec -i -e KIKOERU_CONFIG=/config/config.toml "$name" \
  kikoeru-neo user create --admin admin --password-stdin
test "$(curl -sS -o /dev/null -w '%{http_code}' "http://$port/api/ready")" = 200
test "$(docker exec "$name" stat -c '%u:%g:%a' /data)" = 1000:1000:700
test "$(docker exec "$name" stat -c '%u:%g:%a' /backup)" = 1000:1000:700
