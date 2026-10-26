#!/usr/bin/env bats
# Exercise the smoke script without a Docker daemon or HTTP server.

setup() {
  here=$(cd "$(dirname "$BATS_TEST_FILENAME")" && pwd)
  : "${SMOKE:=$here/../docker-smoke.sh}"
  export FAKE_STATE=$BATS_TEST_TMPDIR
  mkdir -p "$FAKE_STATE/bin" "$FAKE_STATE/tmp"
  export TMPDIR=$FAKE_STATE/tmp
  export PATH="$FAKE_STATE/bin:$PATH"
  cat > "$FAKE_STATE/bin/docker" <<'EOF'
#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$FAKE_STATE/docker.log"
case $1 in
  run) ;;
  inspect)
    if [ "${FAKE_STAYS_STARTING:-false}" = true ] || [ ! -f "$FAKE_STATE/starting" ]; then
      touch "$FAKE_STATE/starting"
      echo starting
    else
      touch "$FAKE_STATE/healthy"
      echo healthy
    fi
    ;;
  port) echo 127.0.0.1:18888 ;;
  exec)
    case " $* " in
      *" user create "*)
        if [ ! -f "$FAKE_STATE/ready" ]; then
          echo "user create ran before readiness" >&2
          exit 1
        fi
        cat > "$FAKE_STATE/password"
        touch "$FAKE_STATE/user-created"
        ;;
      *" stat "*) echo 1000:1000:700 ;;
      *) exit 2 ;;
    esac
    ;;
  logs) echo "synthetic container logs" ;;
  rm) ;;
  volume) [ "$2" = rm ] ;;
  *) exit 2 ;;
esac
EOF
  cat > "$FAKE_STATE/bin/curl" <<'EOF'
#!/bin/sh
set -eu
[ -f "$FAKE_STATE/healthy" ]
case "$*" in
  *http://127.0.0.1:18888/api/ready) ;;
  *) exit 2 ;;
esac
if [ "${FAKE_READY_STATUS:-200}" = 200 ]; then
  touch "$FAKE_STATE/ready"
fi
printf '%s' "${FAKE_READY_STATUS:-200}"
EOF
  cat > "$FAKE_STATE/bin/sleep" <<'EOF'
#!/bin/sh
exit 0
EOF
  chmod +x "$FAKE_STATE/bin/"*
}

assert_cleanup() {
  [[ "$output" == *"synthetic container logs"* ]]
  grep -q '^rm -fv neo-smoke-' "$FAKE_STATE/docker.log"
  grep -q '^volume rm neo-smoke-.*-data neo-smoke-.*-backup$' "$FAKE_STATE/docker.log"
  [ -z "$(ls -A "$TMPDIR")" ]
}

@test "waits for health and HTTP readiness before creating the administrator" {
  run bash "$SMOKE" kikoeru-neo:test
  echo "$output"
  [ "$status" -eq 0 ]
  [ -f "$FAKE_STATE/user-created" ]
  [ "$(cat "$FAKE_STATE/password")" = synthetic-admin-password ]
  grep -q ' stat .* /data$' "$FAKE_STATE/docker.log"
  grep -q ' stat .* /backup$' "$FAKE_STATE/docker.log"
  assert_cleanup
}

@test "health timeout prints logs and cleans up without creating an administrator" {
  export FAKE_STAYS_STARTING=true
  run bash "$SMOKE" kikoeru-neo:test
  [ "$status" -ne 0 ]
  [[ "$(cat "$FAKE_STATE/docker.log")" != *" user create "* ]]
  assert_cleanup
}

@test "failed HTTP readiness prints logs and cleans up without creating an administrator" {
  export FAKE_READY_STATUS=503
  run bash "$SMOKE" kikoeru-neo:test
  [ "$status" -ne 0 ]
  [[ "$(cat "$FAKE_STATE/docker.log")" != *" user create "* ]]
  assert_cleanup
}
