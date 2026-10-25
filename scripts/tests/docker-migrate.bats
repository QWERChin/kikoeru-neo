#!/usr/bin/env bats
# Tests for scripts/docker-migrate.sh against a fake docker. MIGRATE points
# at the script under test, the packaged one in the flake check.

setup() {
  here=$(cd "$(dirname "$BATS_TEST_FILENAME")" && pwd)
  : "${MIGRATE:=$here/../docker-migrate.sh}"
  work=$BATS_TEST_TMPDIR
  export FAKE_LOG=$work/docker.log
  export DOCKER=$here/fake-docker
  : >"$FAKE_LOG"
  cd "$work"
}

# Whatever a test does, the old container is never started.
teardown() {
  while read -r command rest; do
    case $command in
      start | restart | compose | up | unpause)
        echo "docker $command $rest" >&2
        return 1
        ;;
    esac
    case " $rest " in
      *" up "* | *" start "*) [ "$command" = run ] || {
        echo "docker $command $rest" >&2
        return 1
      } ;;
    esac
  done <"$FAKE_LOG"
}

# Bats runs each test under `set -e`, which ignores a failing `! cmd`
# unless it is the last command, so negative checks go through refute,
# which fails the test itself.
refute() {
  if "$@"; then
    echo "expected this to fail: $*" >&2
    return 1
  fi
}

migrate() {
  run "$MIGRATE" --from-container old --image kikoeru-neo:test --out "$work/neo" "$@"
}

# The arguments of the docker run that imports.
import_call() {
  while read -r line; do
    case " $line " in
      " run "*" migrate "*) echo "$line" ;;
    esac
  done <"$FAKE_LOG"
}

# The flake check sets MIGRATE_PACKAGED for the packaged script, whose
# PATH must bring every tool it uses except docker.
@test "the packaged script needs nothing but docker on PATH" {
  [ -n "${MIGRATE_PACKAGED:-}" ] || skip "only for the packaged script"
  export FAKE_PATH=$PATH
  FAKE_MOUNTS=/usr/src/kikoeru/sqlite run env PATH=/var/empty "$MIGRATE" \
    --from-container old --image kikoeru-neo:test --out "$work/neo"
  [ "$status" -eq 0 ]
  [ -s "$work/neo/config.toml" ]
}

# Old mounts under the old data directory come in with --volumes-from. At
# /data, the old /data/sqlite would land inside the new library.
@test "the new library is written where no old mount reaches" {
  FAKE_ENV='KIKOERU_DATA_DIR=/data' FAKE_MOUNTS='/data/sqlite|/data/config' migrate
  [ "$status" -eq 0 ]
  call=$(import_call)
  [[ " $call " == *" -v kikoeru-neo-data:/kikoeru-neo-import "* ]]
  [[ " $call " != *":/data "* ]]
  [[ " $call " == *" migrate --data-dir /kikoeru-neo-import --source /data "* ]]
  # The volume gets the image's /data owner before the import writes.
  grep -q -- '-v kikoeru-neo-data:/data --entrypoint /bin/true kikoeru-neo:test' "$FAKE_LOG"
}

@test "refuses when an old mount reaches the import path" {
  FAKE_MOUNTS='/kikoeru-neo-import/x' migrate
  [ "$status" -ne 0 ]
  [ -z "$(import_call)" ]
}

@test "OpenList password files point at /data in config.toml" {
  export FAKE_CONFIG='[[roots]]|id = "cloud"|password_file = "/kikoeru-neo-import/secrets/openlist-cloud.password"|# password_file = "/kikoeru-neo-import/secrets/openlist-x.password"'
  migrate
  [ "$status" -eq 0 ]
  grep -q '^password_file = "/data/secrets/openlist-cloud.password"$' "$work/neo/config.toml"
  grep -q '^# password_file = "/data/secrets/openlist-x.password"$' "$work/neo/config.toml"
  refute grep -q kikoeru-neo-import "$work/neo/config.toml"
}

# A relative media path resolves against the source, which is the old data
# directory, where --volumes-from puts the old media mount.
@test "the source is the old data directory, not the stage" {
  FAKE_MOUNTS='/usr/src/kikoeru/sqlite|/usr/src/kikoeru/VoiceWork' migrate
  [ "$status" -eq 0 ]
  call=$(import_call)
  [[ " $call " == *" --source /usr/src/kikoeru "* ]]
  [[ " $call " != *" --source /old "* ]]
}

@test "an unreadable old data directory is replaced by an empty one" {
  FAKE_MOUNTS=/usr/src/kikoeru FAKE_ROOT_READABLE=false migrate
  [ "$status" -eq 0 ]
  call=$(import_call)
  [[ " $call " == *"/root:/usr/src/kikoeru "* ]]
  [[ " $call " == *"/old/db:/usr/src/kikoeru/sqlite:ro "* ]]
  [[ " $call " == *" --source /usr/src/kikoeru "* ]]
}

@test "a readable old data directory stays in place" {
  FAKE_MOUNTS=/usr/src/kikoeru migrate
  [ "$status" -eq 0 ]
  [[ " $(import_call) " != *"/root:/usr/src/kikoeru "* ]]
}

@test "a library under /data gets a warning in the compose snippet" {
  export FAKE_CONFIG='[[roots]]|id = "voice"|path = "/data/VoiceWork"'
  FAKE_ENV='KIKOERU_DATA_DIR=/data' \
    FAKE_MOUNTS_FULL="$(printf 'bind\t/srv/voice\t/data/VoiceWork\t')" migrate
  [ "$status" -eq 0 ]
  [[ $output == *"/data/VoiceWork is inside /data"* ]]
}

@test "refuses a running container and touches nothing else" {
  FAKE_RUNNING=true migrate
  [ "$status" -ne 0 ]
  [[ $output == *"is running"* ]]
  [ "$(wc -l <"$FAKE_LOG")" -eq 1 ]
}

@test "imports from a staged copy without a terminal" {
  FAKE_MOUNTS=/usr/src/kikoeru/sqlite migrate --root Old=old </dev/null
  [ "$status" -eq 0 ]
  call=$(import_call)
  # The media libraries stay reachable at their old paths.
  [[ " $call " == *" --volumes-from old:ro "* ]]
  # The copies go back at the old paths, and the source is the old data
  # directory, so paths in config.json resolve as they did.
  [[ " $call " == *"/old/db:/usr/src/kikoeru/sqlite:ro "* ]]
  [[ " $call " == *"/old/config:/usr/src/kikoeru/config:ro "* ]]
  [[ " $call " == *"/old/covers:/usr/src/kikoeru/covers:ro "* ]]
  [[ " $call " == *" migrate --data-dir /kikoeru-neo-import --source /usr/src/kikoeru --root Old=old --yes "* ]]
  [[ " $call " != *"--map-path"* ]]
  [[ " $call " != *" -t "* ]]
  [[ " $call " != *" -it "* ]]
  [ "$(stat -c %a "$work/neo/config.toml")" = 644 ]
  grep -q 'id = "old"' "$work/neo/config.toml"
  grep -Eq '^volume create --label kikoeru-neo.migrate=[0-9a-f]{32} kikoeru-neo-data$' "$FAKE_LOG"
  refute grep -q '^volume rm' "$FAKE_LOG"
  grep -q -- '--entrypoint /bin/rm kikoeru-neo:test -f /data/config.toml' "$FAKE_LOG"
  # The stage is gone afterwards.
  [ -z "$(ls -d "$work"/kikoeru-neo-stage.* 2>/dev/null)" ]
}

@test "passes -t only when stdin and stdout are terminals" {
  command -v script >/dev/null || skip "script(1) is not available"
  FAKE_MOUNTS=/usr/src/kikoeru script -qec \
    "'$MIGRATE' --from-container old --image kikoeru-neo:test --out '$work/neo'" /dev/null
  call=$(import_call)
  [[ " $call " == *" run --rm -i -t "* ]]
  [[ " $call " != *" --yes "* ]]
}

@test "stdin from a terminal alone is not enough for -t" {
  command -v script >/dev/null || skip "script(1) is not available"
  FAKE_MOUNTS=/usr/src/kikoeru script -qec \
    "'$MIGRATE' --from-container old --image kikoeru-neo:test --out '$work/neo' >'$work/stdout'" /dev/null
  call=$(import_call)
  [[ " $call " != *" -t "* ]]
  [[ " $call " == *" --yes "* ]]
}

@test "KIKOERU_DATA_DIR wins over WorkingDir" {
  FAKE_ENV='PATH=/bin|KIKOERU_DATA_DIR=/data/kikoeru/' FAKE_WORKDIR=/app migrate
  [ "$status" -eq 0 ]
  grep -q '^cp old:/data/kikoeru/sqlite/db.sqlite3 ' "$FAKE_LOG"
  [[ " $(import_call) " == *"/old/db:/data/kikoeru/sqlite:ro "* ]]
  [[ " $(import_call) " == *" --source /data/kikoeru "* ]]
}

@test "WorkingDir is used when KIKOERU_DATA_DIR is not set" {
  FAKE_WORKDIR=/app migrate
  [ "$status" -eq 0 ]
  grep -q '^cp old:/app/sqlite/db.sqlite3 ' "$FAKE_LOG"
}

# Only sqlite/ is a mount here. config.json and covers/ sit in the old
# container's own layer, which only docker cp can read.
@test "copies config.json and covers that are not on any mount" {
  FAKE_MOUNTS=/usr/src/kikoeru/sqlite \
    FAKE_MISSING="/usr/src/kikoeru/sqlite/db.sqlite3-journal /usr/src/kikoeru/sqlite/db.sqlite3-wal" \
    migrate
  [ "$status" -eq 0 ]
  grep -q '^cp old:/usr/src/kikoeru/config/config.json ' "$FAKE_LOG"
  grep -q '^cp old:/usr/src/kikoeru/covers ' "$FAKE_LOG"
  grep -q '^cp old:/usr/src/kikoeru/sqlite/db.sqlite3 ' "$FAKE_LOG"
  refute grep -q '^cp old:/usr/src/kikoeru/lyrics' "$FAKE_LOG"
  call=$(import_call)
  [[ " $call " == *"/old/config:/usr/src/kikoeru/config:ro "* ]]
  [[ " $call " == *"/old/covers:/usr/src/kikoeru/covers:ro "* ]]
}

@test "copies the database and covers from the folders config.json names" {
  export FAKE_OLD_CONFIG='{"dbUseDefaultPath": false, "databaseFolderDir": "\/srv\/db\/", "coverUseDefaultPath":false,"coverFolderDir":"art"}'
  migrate
  [ "$status" -eq 0 ]
  grep -q '^cp old:/srv/db/db.sqlite3 ' "$FAKE_LOG"
  grep -q '^cp old:/usr/src/kikoeru/art ' "$FAKE_LOG"
  refute grep -q '^cp old:/usr/src/kikoeru/sqlite/db.sqlite3 ' "$FAKE_LOG"
  [[ " $(import_call) " == *"/old/db:/srv/db:ro "* ]]
  [[ " $(import_call) " == *"/old/covers:/usr/src/kikoeru/art:ro "* ]]
}

@test "a relative database folder goes back at its old place" {
  export FAKE_OLD_CONFIG='{"dbUseDefaultPath":false,"databaseFolderDir":"db/"}'
  migrate
  [ "$status" -eq 0 ]
  grep -q '^cp old:/usr/src/kikoeru/db/db.sqlite3 .*/old/db/db.sqlite3$' "$FAKE_LOG"
  [[ " $(import_call) " == *"/old/db:/usr/src/kikoeru/db:ro "* ]]
}

@test "imports when the database and covers share a directory" {
  export FAKE_OLD_CONFIG='{"dbUseDefaultPath":false,"databaseFolderDir":"/legacy-store","coverUseDefaultPath":false,"coverFolderDir":"/legacy-store"}'
  FAKE_MOUNTS=/legacy-store migrate
  echo "$output"
  [ "$status" -eq 0 ]
  [[ " $(import_call) " == *":/legacy-store:ro "* ]]
}

@test "shared copies win over old single-file mounts without duplicate targets" {
  export FAKE_OLD_CONFIG='{"dbUseDefaultPath":false,"databaseFolderDir":"/legacy-store","coverUseDefaultPath":false,"coverFolderDir":"/legacy-store"}'
  FAKE_MOUNTS='/legacy-store/db.sqlite3|/legacy-store/RJ123456_img_main.jpg' migrate
  echo "$output"
  [ "$status" -eq 0 ]
  call=$(import_call)
  [[ " $call " == *":/legacy-store:ro "* ]]
  [[ " $call " == *":/legacy-store/db.sqlite3:ro "* ]]
  [[ " $call " == *":/legacy-store/RJ123456_img_main.jpg:ro "* ]]
}

@test "refuses a relative folder that leaves the data directory" {
  export FAKE_OLD_CONFIG='{"dbUseDefaultPath":false,"databaseFolderDir":"../db"}'
  migrate
  [ "$status" -ne 0 ]
  [[ $output == *"../db"* ]]
  [ -z "$(import_call)" ]
}

@test "the default folders win while config.json says to use them" {
  export FAKE_OLD_CONFIG='{"dbUseDefaultPath":true,"databaseFolderDir":"/usr/src/kikoeru/sqlite","coverUseDefaultPath":true,"coverFolderDir":"/elsewhere"}'
  migrate
  [ "$status" -eq 0 ]
  grep -q '^cp old:/usr/src/kikoeru/sqlite/db.sqlite3 ' "$FAKE_LOG"
  grep -q '^cp old:/usr/src/kikoeru/covers ' "$FAKE_LOG"
}

@test "refuses a config.json path it cannot unescape" {
  export FAKE_OLD_CONFIG='{"dbUseDefaultPath":false,"databaseFolderDir":"C:\\kikoeru\\sqlite"}'
  migrate
  [ "$status" -ne 0 ]
  [[ $output == *"databaseFolderDir"* ]]
  [ -z "$(import_call)" ]
  refute grep -q '^volume create' "$FAKE_LOG"
}

@test "a missing old database stops before the volume is created" {
  FAKE_MISSING=/usr/src/kikoeru/sqlite/db.sqlite3 migrate --stage-dir "$work"
  [ "$status" -ne 0 ]
  [[ $output == *"--data-root"* ]]
  refute grep -q '^volume create' "$FAKE_LOG"
  [ -z "$(import_call)" ]
  [ -z "$(ls -d "$work"/kikoeru-neo-stage.* 2>/dev/null)" ]
}

# Another run, or anyone else, may create the volume between the check and
# the create, and docker volume create then returns that volume.
@test "claims only a volume carrying this run's label" {
  FAKE_VOLUME_FOREIGN=true migrate
  [ "$status" -ne 0 ]
  [[ $output == *"not created here"* ]]
  [ -z "$(import_call)" ]
  refute grep -q '^volume rm' "$FAKE_LOG"
  refute grep -q -- '--entrypoint /bin/true' "$FAKE_LOG"
}

# The old container mounts single files. --volumes-from stacks them on top
# of the staged folders, so the copies are bound at those paths too.
@test "copies win over old single-file mounts" {
  FAKE_MOUNTS='/usr/src/kikoeru/config/config.json|/usr/src/kikoeru/sqlite/db.sqlite3|/usr/src/kikoeru/covers/RJ123456_img_main.jpg|/usr/src/kikoeru/sqlite/lyrics' migrate
  [ "$status" -eq 0 ]
  call=$(import_call)
  [[ " $call " == *"/old/config/config.json:/usr/src/kikoeru/config/config.json:ro "* ]]
  [[ " $call " == *"/old/db/db.sqlite3:/usr/src/kikoeru/sqlite/db.sqlite3:ro "* ]]
  [[ " $call " == *"/old/covers/RJ123456_img_main.jpg:/usr/src/kikoeru/covers/RJ123456_img_main.jpg:ro "* ]]
  # Nothing was copied for lyrics, and the import does not read it.
  [[ " $call " != *":/usr/src/kikoeru/sqlite/lyrics"* ]]
}

@test "refuses a volume that already exists" {
  FAKE_VOLUME_EXISTS=true migrate
  [ "$status" -ne 0 ]
  [[ $output == *"already exists"* ]]
  [ -z "$(import_call)" ]
  refute grep -q '^volume rm' "$FAKE_LOG"
}

@test "refuses to overwrite an existing config.toml" {
  mkdir -p "$work/neo" && echo keep >"$work/neo/config.toml"
  FAKE_MOUNTS=/usr/src/kikoeru migrate
  [ "$status" -ne 0 ]
  [ "$(cat "$work/neo/config.toml")" = keep ]
  [ -z "$(import_call)" ]
}

@test "a failed import removes the volume it created" {
  FAKE_MOUNTS=/usr/src/kikoeru FAKE_MIGRATE_EXIT=1 migrate
  [ "$status" -ne 0 ]
  grep -q '^volume rm kikoeru-neo-data$' "$FAKE_LOG"
  [ ! -e "$work/neo/config.toml" ]
}

@test "prints a mount for each media library in config.toml" {
  export FAKE_CONFIG='[[roots]]|id = "voice"|path = "/usr/src/kikoeru/VoiceWork"|[[roots]]|id = "more"|path = "/media/more"|# [[roots]]|# path = "/gone"'
  FAKE_MOUNTS_FULL="$(printf 'bind\t/srv/old\t/usr/src/kikoeru\t|volume\t/var/lib/docker/volumes/media/_data\t/media\tmedia')" \
    migrate
  [ "$status" -eq 0 ]
  # Narrowed to the library, so the old data folder is not mounted.
  [[ $output == *"- /srv/old/VoiceWork:/usr/src/kikoeru/VoiceWork:ro"* ]]
  [[ $output != *"- /srv/old:"* ]]
  [[ $output == *"- media:/media:ro"* ]]
  [[ $output != *"/gone"* ]]
  [[ $output != *"mount the other media libraries"* ]]
  [[ $output == *"data: { name: kikoeru-neo-data, external: true }"* ]]
}

@test "the deepest old mount of a library wins" {
  export FAKE_CONFIG='[[roots]]|id = "voice"|path = "/app/VoiceWork"'
  FAKE_MOUNTS_FULL="$(printf 'bind\t/srv/app\t/app\t|bind\t/srv/voice\t/app/VoiceWork\t')" migrate
  [ "$status" -eq 0 ]
  [[ $output == *"- /srv/voice:/app/VoiceWork:ro"* ]]
  [[ $output != *"/srv/app/VoiceWork"* ]]
  [ "$(grep -c ':/app/VoiceWork:ro' <<<"$output")" -eq 1 ]
}

@test "mounts inside a library come along" {
  export FAKE_CONFIG='[[roots]]|id = "media"|path = "/media"'
  FAKE_MOUNTS_FULL="$(printf 'bind\t/srv/media\t/media\t|bind\t/srv/extra\t/media/extra\t|volume\t/v/_data\t/media/vol\tvol')" migrate
  [ "$status" -eq 0 ]
  [[ $output == *"- /srv/media:/media:ro"* ]]
  [[ $output == *"- /srv/extra:/media/extra:ro"* ]]
  [[ $output == *"- vol:/media/vol:ro"* ]]
}

@test "says so when a media library is on no old mount" {
  export FAKE_CONFIG='[[roots]]|id = "voice"|path = "/elsewhere"'
  migrate
  [ "$status" -eq 0 ]
  [[ $output == *"mount the other media libraries"* ]]
}

@test "never reads through a link the old container planted" {
  # A config.json that is a link to a file on this host. Followed, it
  # would move the database copy to /evil.
  printf '%s\n' '{"dbUseDefaultPath": false, "databaseFolderDir": "/evil"}' >"$work/host-file"
  FAKE_LINKS=/usr/src/kikoeru/config/config.json FAKE_LINK_TARGET=$work/host-file migrate </dev/null
  [ "$status" -eq 0 ]
  refute grep -q "/evil" "$FAKE_LOG"
}

@test "refuses a database that is a link" {
  FAKE_LINKS=/usr/src/kikoeru/sqlite/db.sqlite3 migrate </dev/null
  [ "$status" -ne 0 ]
  [[ $output == *"not a regular file"* ]]
}
