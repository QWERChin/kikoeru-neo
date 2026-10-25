#!/usr/bin/env bash
# kikoeru-neo-docker-migrate: imports a stopped old kikoeru container into a
# new Docker volume for the kikoeru-neo image. book/src/deployment.md
# ("从旧 kikoeru 容器迁移") describes how to use it.
#
# It only needs docker (or podman), coreutils and findutils. It never starts the old
# container: the old image's latest tag migrates the old database on start.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: kikoeru-neo-docker-migrate --from-container NAME --image IMAGE [options]

Imports the data of a stopped old kikoeru container into a new volume and
writes the media libraries into OUT/config.toml.

  --from-container NAME  the old kikoeru container; it must be stopped
  --image IMAGE          the kikoeru-neo image, such as kikoeru-neo:0.1.0
  --volume NAME          the new data volume (default kikoeru-neo-data);
                         it must not exist yet
  --out DIR              where config.toml goes (default ./neo)
  --stage-dir DIR        where the old data is copied when the container
                         cannot share it (default .)
  --data-root PATH       the old data directory inside the old container,
                         when it is not found on its own
  --root OLD=NEW         the new ID of an old media library; repeatable
  --map-path OLD=NEW     where a folder named in the old config is now,
                         inside the new container; repeatable
  -h, --help             show this help

Set DOCKER=podman to use podman. Without a terminal the import takes every
default (migrate --yes) and sets no passwords.
EOF
}

die() {
  printf 'kikoeru-neo-docker-migrate: %s\n' "$*" >&2
  exit 1
}

container=""
image=""
volume="kikoeru-neo-data"
out="./neo"
stage_parent="."
data_root=""
forward=()

while [ $# -gt 0 ]; do
  case $1 in
    --from-container | --image | --volume | --out | --stage-dir | --data-root | --root | --map-path)
      [ $# -ge 2 ] || die "$1 needs a value"
      case $1 in
        --from-container) container=$2 ;;
        --image) image=$2 ;;
        --volume) volume=$2 ;;
        --out) out=$2 ;;
        --stage-dir) stage_parent=$2 ;;
        --data-root) data_root=$2 ;;
        --root | --map-path) forward+=("$1" "$2") ;;
      esac
      shift 2
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *) die "unknown argument $1 (see --help)" ;;
  esac
done
[ -n "$container" ] || die "--from-container is required (see --help)"
[ -n "$image" ] || die "--image is required (see --help)"

if [ -n "${DOCKER:-}" ]; then
  docker=$DOCKER
elif command -v docker >/dev/null; then
  docker=docker
elif command -v podman >/dev/null; then
  docker=podman
else
  die "neither docker nor podman is on PATH"
fi

# Step 1: the old container must exist and be stopped. Its state is only
# read here; nothing below ever starts it.
running=$("$docker" inspect --type container -f '{{.State.Running}}' "$container") ||
  die "cannot inspect container $container"
if [ "$running" != false ]; then
  die "container $container is running; stop it first (docker stop $container) and do not start it again"
fi

# Step 2: the old data directory, as the old server chose it.
if [ -z "$data_root" ]; then
  while IFS= read -r line || [ -n "$line" ]; do
    case $line in
      KIKOERU_DATA_DIR=?*) data_root=${line#KIKOERU_DATA_DIR=} ;;
    esac
  done < <("$docker" inspect --type container -f '{{range .Config.Env}}{{println .}}{{end}}' "$container")
fi
if [ -z "$data_root" ]; then
  data_root=$("$docker" inspect --type container -f '{{.Config.WorkingDir}}' "$container")
  [ "$data_root" != / ] || data_root=""
fi
[ -n "$data_root" ] || data_root=/usr/src/kikoeru
while [ "$data_root" != / ] && [ "${data_root%/}" != "$data_root" ]; do
  data_root=${data_root%/}
done
case $data_root in
  /*) ;;
  *) die "the old data directory $data_root is not an absolute path; pass --data-root" ;;
esac

# Step 5 comes before any copying: an existing volume may hold a library.
if "$docker" volume inspect "$volume" >/dev/null 2>&1; then
  die "volume $volume already exists; pick another name with --volume, or remove it if it is a failed earlier attempt"
fi
[ ! -e "$out/config.toml" ] || die "$out/config.toml already exists"

stage=""
created_volume=false
done_ok=false
cleanup() {
  if [ -n "$stage" ]; then
    rm -rf -- "$stage"
  fi
  if [ "$created_volume" = true ] && [ "$done_ok" = false ]; then
    "$docker" volume rm "$volume" >/dev/null 2>&1 ||
      printf 'kikoeru-neo-docker-migrate: could not remove volume %s; remove it before trying again\n' "$volume" >&2
  fi
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# Copies one path out of the old container. A missing optional path is
# skipped. Any other failure stops the script.
copy_out() {
  local from=$1 to=$2 required=$3 err
  err=$stage/cp.err
  if "$docker" cp "$container:$from" "$to" 2>"$err"; then
    # docker cp keeps symlinks as links, and the old container controls
    # where they point. Reading or chmod-ing through one would reach files
    # on this host, so every link in the copy is removed.
    find "$to" -type l -exec rm -f -- {} +
    return 0
  fi
  case $(tr '[:upper:]' '[:lower:]' <"$err") in
    *"no such file"* | *"could not find"*)
      [ "$required" = false ] || die "$from is not in container $container; pass --data-root"
      ;;
    *) die "docker cp $from failed: $(cat "$err")" ;;
  esac
}

# The value of a top-level string field in the old config.json. kikoeru
# writes plain container paths, so only \/ is unescaped, and any other
# escape is refused rather than guessed at.
json_string() {
  local re='"'$1'"[[:space:]]*:[[:space:]]*"(([^"\\]|\\.)*)"' value
  [[ $old_config =~ $re ]] || return 0
  value=${BASH_REMATCH[1]//'\/'/\/}
  case $value in
    *\\*) die "config.json gives $1 with an escape this script cannot read; pass --data-root" ;;
  esac
  printf '%s' "$value"
}

json_false() {
  local re='"'$1'"[[:space:]]*:[[:space:]]*false'
  [[ $old_config =~ $re ]]
}

# A data folder from config.json as an absolute path in the old container.
# The old server resolves a relative one against its data directory. . and
# .. would make the mounts below land somewhere else, so they are refused.
old_folder() {
  local folder=$1
  case $folder in /*) ;; *) folder=$data_root/$folder ;; esac
  case $folder/ in
    */../* | */./*) die "config.json gives the folder $1, which this script cannot place; pass --data-root" ;;
  esac
  printf '%s' "${folder%/}"
}

# Steps 3 and 4: docker cp reads a stopped container's own layer and its
# mounts alike, so the import's inputs are always copied, wherever the old
# server kept them. Only config.json, the database with its journals and
# the covers are copied. lyrics and caches can be gigabytes and are not
# imported. The stage stays 0700 because config.json holds the old
# secrets. Only the inner directory, which the container mounts, is opened
# up for its uid 1000.
[ -d "$stage_parent" ] || die "--stage-dir $stage_parent does not exist"
stage=$(mktemp -d "$stage_parent/kikoeru-neo-stage.XXXXXX")
stage=$(cd "$stage" && pwd)
echo "copying the old database and covers into $stage"
mkdir -p "$stage/old/config" "$stage/old/db"
copy_out "$data_root/config/config.json" "$stage/old/config/config.json" false
old_config=""
if [ -f "$stage/old/config/config.json" ]; then
  old_config=$(cat "$stage/old/config/config.json")
fi
db_dir=$data_root/sqlite
covers_dir=$data_root/covers
if json_false dbUseDefaultPath; then
  folder=$(json_string databaseFolderDir)
  [ -z "$folder" ] || db_dir=$(old_folder "$folder")
fi
if json_false coverUseDefaultPath; then
  folder=$(json_string coverFolderDir)
  [ -z "$folder" ] || covers_dir=$(old_folder "$folder")
fi
copy_out "$db_dir/db.sqlite3" "$stage/old/db/db.sqlite3" true
[ -f "$stage/old/db/db.sqlite3" ] ||
  die "$db_dir/db.sqlite3 in container $container is not a regular file"
copy_out "$db_dir/db.sqlite3-journal" "$stage/old/db/db.sqlite3-journal" false
copy_out "$db_dir/db.sqlite3-wal" "$stage/old/db/db.sqlite3-wal" false
copy_out "$covers_dir" "$stage/old/covers" false
chmod -R u+rwX,go+rX "$stage/old"
db_copy=$stage/old/db
if [ "$covers_dir" = "$db_dir" ] && [ -d "$stage/old/covers" ]; then
  # Docker rejects two explicit mounts at the same path. Keep the checked
  # database and its journals alongside the covers in one read-only mount.
  cp -a "$db_copy/." "$stage/old/covers/"
  db_copy=$stage/old/covers
fi

# The copies go back at the paths the old server used, on top of the old
# mounts from --volumes-from: an explicit bind replaces an inherited mount
# at the same path, and deeper mounts are stacked after shallower ones. The
# source is then the old data directory itself, so absolute and relative
# paths in config.json, media libraries included, resolve as they did for
# the old server.
binds=()
# The import runs as uid 1000 and reads the source directory itself. When
# the old data directory is a root-owned 0700 mount, or exists only in the
# old container's own layer, an empty directory of the stage takes its
# place. Media libraries inside it would be unreadable for the new server
# too. It is writable so that the mounts below can make their mount points
# in it.
if ! "$docker" run --rm --network none --user 1000:1000 --volumes-from "$container:ro" \
  --entrypoint /bin/test "$image" -r "$data_root" -a -x "$data_root"; then
  mkdir -p "$stage/root"
  chmod 0755 "$stage/root"
  binds+=(-v "$stage/root:$data_root")
fi
binds+=(-v "$db_copy:$db_dir:ro")
if [ -f "$stage/old/config/config.json" ]; then
  binds+=(-v "$stage/old/config:$data_root/config:ro")
fi
if [ "$covers_dir" != "$db_dir" ] && [ -d "$stage/old/covers" ]; then
  binds+=(-v "$stage/old/covers:$covers_dir:ro")
fi

# The new library is written at a path no old mount touches. Mounted at
# /data, inherited mounts under the old data directory, such as
# /data/sqlite when it was /data, would land inside it, and the import
# refuses a target that is not empty.
target=/kikoeru-neo-import
read_dirs=("$db_dir" "$data_root/config")
read_copies=("$db_copy" "$stage/old/config")
if [ "$covers_dir" != "$db_dir" ]; then
  read_dirs+=("$covers_dir")
  read_copies+=("$stage/old/covers")
fi
while IFS= read -r dest || [ -n "$dest" ]; do
  dest=${dest%/}
  [ -n "$dest" ] || continue
  case $target/ in
    "$dest"/*) die "an old mount at $dest covers $target, where the import writes" ;;
  esac
  case $dest/ in
    "$target"/*) die "an old mount at $dest is inside $target, where the import writes" ;;
  esac
  # An inherited mount inside a folder bound above, such as config.json or
  # db.sqlite3 mounted as a single file, is stacked on top of the copy and
  # would hide it. The copy of that path is bound there as well.
  for i in "${!read_dirs[@]}"; do
    dir=${read_dirs[i]} copy=${read_copies[i]}
    case $dest in
      "$dir"/*)
        if [ -e "$copy${dest#"$dir"}" ]; then
          binds+=(-v "$copy${dest#"$dir"}:$dest:ro")
        fi
        ;;
    esac
  done
done < <("$docker" inspect --type container -f '{{range .Mounts}}{{println .Destination}}{{end}}' "$container")

# docker volume create returns a volume of that name if one appeared since
# the check above, so the volume is only ours when it carries this run's
# label. Anything else is refused and left alone.
token=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
"$docker" volume create --label "kikoeru-neo.migrate=$token" "$volume" >/dev/null
label=$("$docker" volume inspect -f '{{index .Labels "kikoeru-neo.migrate"}}' "$volume")
[ "$label" = "$token" ] ||
  die "volume $volume appeared while this script ran; it was not created here and is left as it is"
created_volume=true
# The first mount at the image's /data copies its owner and mode, uid 1000
# and 0700, onto the empty volume. A path the image lacks would leave it
# root-owned, and the import, which runs as uid 1000, could not write.
"$docker" run --rm --network none -v "$volume:/data" --entrypoint /bin/true "$image"

# Step 6: the import. -t only with a terminal on both ends, otherwise
# docker refuses to start. Without one, migrate takes every default.
run=(run --rm -i)
interactive=false
if [ -t 0 ] && [ -t 1 ]; then
  run+=(-t)
  interactive=true
fi
run+=(--no-healthcheck --volumes-from "$container:ro" -v "$volume:$target" "${binds[@]}")
cmd=(migrate --data-dir "$target" --source "$data_root" "${forward[@]}")
[ "$interactive" = true ] || cmd+=(--yes)
"$docker" "${run[@]}" "$image" "${cmd[@]}"

# migrate writes the media libraries into /data/config.toml, but the image
# reads /config/config.toml, a read-only host file. Reading it through the
# container's stdout leaves the host copy owned by the user running this
# script, which also holds under rootless Docker, where a bind-mounted
# output directory would get a file the user cannot read.
mkdir -p "$out"
"$docker" run --rm --network none -v "$volume:/data:ro" --entrypoint /bin/cat "$image" \
  /data/config.toml >"$out/config.toml.part" ||
  die "the import finished, but /data/config.toml could not be read from volume $volume"
# migrate wrote OpenList password files as paths in its data directory,
# which the server will see at /data.
config=$(cat "$out/config.toml.part")
from="\"$target/" to='"/data/'
printf '%s\n' "${config//"$from"/"$to"}" >"$out/config.toml.part"
# It holds no secrets, and the container's uid 1000 must be able to read it.
chmod 0644 "$out/config.toml.part"
mv "$out/config.toml.part" "$out/config.toml"
# Only /config/config.toml takes effect, so the copy in the volume would
# only mislead whoever edits it later.
"$docker" run --rm --network none -v "$volume:/data" --entrypoint /bin/rm "$image" -f /data/config.toml
done_ok=true

echo
echo "imported into volume $volume; the media libraries are in $out/config.toml"
if [ "$interactive" = false ]; then
  echo "no passwords were set; set one for each user before logging in:"
  echo "  docker run --rm -it -v $volume:/data $image user set-password <name>"
fi
echo
echo "compose.yaml (check the media mounts and the TODO lines in config.toml):"
cat <<EOF
services:
  kikoeru-neo:
    image: $image
    restart: unless-stopped
    stop_grace_period: 30s
    read_only: true
    tmpfs: [/tmp]
    cap_drop: [ALL]
    security_opt: ["no-new-privileges:true"]
    ports: ["127.0.0.1:8888:8888"]
    volumes:
      - data:/data
      - type: bind
        source: $out/config.toml
        target: /config/config.toml
        read_only: true
        bind: { create_host_path: false }
EOF
# The local media libraries that migrate wrote into config.toml. TOML
# writes a plain path as "..." and one with a quote or backslash as '...'.
roots=()
while IFS= read -r line || [ -n "$line" ]; do
  case $line in
    'path = "'*'"') line=${line#'path = "'} && roots+=("${line%'"'}") ;;
    "path = '"*"'") line=${line#"path = '"} && roots+=("${line%"'"}") ;;
  esac
done <"$out/config.toml"

# The old container's mounts. Name comes last in the template because it
# is empty for a bind mount, and read would merge an empty field in the
# middle with the tabs around it.
m_type=() m_src=() m_dest=() m_name=()
while IFS=$'\t' read -r type src dest name || [ -n "$type" ]; do
  dest=${dest%/}
  [ -n "$dest" ] || continue
  m_type+=("$type") m_src+=("$src") m_dest+=("$dest") m_name+=("$name")
done < <("$docker" inspect --type container \
  -f '{{range .Mounts}}{{.Type}}{{"\t"}}{{.Source}}{{"\t"}}{{.Destination}}{{"\t"}}{{.Name}}{{println}}{{end}}' "$container")

printed=$'\n'
mount_line() {
  case $printed in
    *$'\n'"$1"$'\n'*) return ;;
  esac
  printed+="$1"$'\n'
  echo "      - $1"
}

# Each library is mounted at the path config.toml names, from the deepest
# old mount that holds it, since Compose keeps only one mount per target.
# A bind mount is narrowed to the library folder, so the old data folder
# next to it, with its secrets, stays out. Mounts inside a library come
# along at their own paths.
unmounted=0
for path in "${roots[@]}"; do
  case $path/ in
    /data/*) echo "      # $path is inside /data, the new data volume; give the library another path here and in config.toml" ;;
  esac
  best=-1
  for i in "${!m_dest[@]}"; do
    case $path in
      "${m_dest[i]}" | "${m_dest[i]}"/*)
        if [ "$best" -lt 0 ] || [ ${#m_dest[i]} -gt ${#m_dest[best]} ]; then
          best=$i
        fi
        ;;
    esac
  done
  if [ "$best" -lt 0 ]; then
    unmounted=$((unmounted + 1))
  elif [ "${m_type[best]}" = volume ]; then
    mount_line "${m_name[best]}:${m_dest[best]}:ro   # volume ${m_name[best]}, declare it external below"
  else
    mount_line "${m_src[best]%/}${path#"${m_dest[best]}"}:$path:ro"
  fi
  for i in "${!m_dest[@]}"; do
    case ${m_dest[i]} in
      "$path"/*) ;;
      *) continue ;;
    esac
    if [ "${m_type[i]}" = volume ]; then
      mount_line "${m_name[i]}:${m_dest[i]}:ro   # volume ${m_name[i]}, declare it external below"
    else
      mount_line "${m_src[i]}:${m_dest[i]}:ro"
    fi
  done
done
if [ "$unmounted" -gt 0 ]; then
  echo "      # mount the other media libraries at the path config.toml gives them"
fi
cat <<EOF
volumes:
  data: { name: $volume, external: true }
EOF
