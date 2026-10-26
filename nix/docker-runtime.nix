# Both image builders use these files, directory permissions and metadata.
{ pkgs, server }:

let
  inherit (pkgs) lib;
  uid = "1000";
  seconds = n: n * 1000000000;

  # Files the server creates, backups included, stay private to the
  # service user whatever umask the container runtime starts with.
  entrypoint = pkgs.writeShellScriptBin "kikoeru-neo-entrypoint" ''
    umask 077
    exec ${lib.getExe server} "$@"
  '';

  # The image has neither curl nor wget. bash is already in the closure
  # through the server's wrapper script, so /dev/tcp costs nothing. The port
  # comes from KIKOERU_LISTEN alone, so a deployment that clears it to use
  # `listen` from the config file runs this with the variable set in the
  # healthcheck's own command, which is why it sits at a fixed path.
  healthcheck = pkgs.writeShellScriptBin "kikoeru-neo-healthcheck" ''
    set -eu
    listen=''${KIKOERU_LISTEN:?KIKOERU_LISTEN is empty, set it in the healthcheck command}
    host=''${listen%:*}
    port=''${listen##*:}
    # A wildcard is probed on the loopback of its own family: with
    # bindv6only=1, [::] accepts no IPv4 connections.
    case $host in
      0.0.0.0) host=127.0.0.1 ;;
      "[::]") host=::1 ;;
    esac
    host=''${host#\[}
    host=''${host%\]}
    # For nix/docker-check.py, which tests the mapping above.
    if [ "''${1-}" = --print-address ]; then
      printf '%s %s\n' "$host" "$port"
      exit 0
    fi
    exec 3<>"/dev/tcp/$host/$port"
    printf 'GET /api/ready HTTP/1.0\r\nHost: localhost\r\n\r\n' >&3
    IFS= read -r -t 5 status <&3
    case $status in
      "HTTP/1."?" 200 "*) exit 0 ;;
      *) echo "/api/ready answered: $status" >&2; exit 1 ;;
    esac
  '';
in
rec {
  inherit entrypoint healthcheck;
  paths = [
    # /bin/kikoeru-neo, for `docker exec ... kikoeru-neo backup`.
    server
    (pkgs.dockerTools.fakeNss.override {
      extraPasswdLines = [ "kikoeru:x:${uid}:${uid}:kikoeru-neo:/data:/bin/false" ];
      extraGroupLines = [ "kikoeru:x:${uid}:" ];
    })
    # Already in the closure. Lets an operator `chown -R` a restored volume.
    pkgs.coreutils
    pkgs.tini
    healthcheck
    entrypoint
  ];
  root = pkgs.buildEnv {
    name = "kikoeru-neo-docker-runtime";
    inherit paths;
    pathsToLink = [ "/bin" "/etc" ];
  };
  # A fresh named volume starts as a copy of the image directory, owner and
  # mode included. /data must be empty because an import only accepts an
  # empty target, and 0700 because the server refuses a data directory that
  # others can write to. backup refuses a destination inside the data
  # directory, so it gets its own volume. SQLite needs a writable /tmp.
  fakeRootCommands = ''
    mkdir data backup tmp config
    chown ${uid}:${uid} data backup
    chmod 0700 data backup
    chmod 1777 tmp
  '';
  config = {
    # serve installs its signal handlers only after migrations, and the
    # other commands install none, so PID 1 must be tini, which forwards
    # signals and reaps ffmpeg children that outlive their parent.
    Entrypoint = [ "/bin/tini" "--" "/bin/kikoeru-neo-entrypoint" ];
    Cmd = [ "serve" "--config" "/config/config.toml" ];
    User = "${uid}:${uid}";
    Env = [
      "KIKOERU_DATA_DIR=/data"
      "KIKOERU_LISTEN=0.0.0.0:8888"
      # serve and user create cache/tmp. SQLite skips a missing directory
      # and falls back to /tmp, which a read-only root mounts as tmpfs in
      # RAM, too small for a VACUUM INTO or an index build on a big library.
      "SQLITE_TMPDIR=/data/cache/tmp"
      # Bound allocator fragmentation from remote transfers, as in the
      # NixOS module. See book/src/testing.md (Chinese) for the rationale.
      "MALLOC_ARENA_MAX=2"
    ];
    ExposedPorts."8888/tcp" = { };
    # The release script reads this to move the floating X.Y tag only
    # forward.
    Labels = {
      "org.opencontainers.image.version" = server.version;
      "org.opencontainers.image.source" = "https://github.com/QWERChin/kikoeru-neo";
      "org.opencontainers.image.licenses" = "GPL-3.0-or-later";
    };
    Volumes = {
      "/data" = { };
      "/backup" = { };
    };
    Healthcheck = {
      Test = [ "CMD" "/bin/kikoeru-neo-healthcheck" ];
      Interval = seconds 30;
      Timeout = seconds 10;
      Retries = 3;
      # A first start migrates the database before /api/ready answers.
      StartPeriod = seconds 120;
      StartInterval = seconds 2;
    };
  };
}
