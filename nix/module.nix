# services.kikoeru-neo, documented in docs/en/install-nixos.md.
self:
{ config, lib, pkgs, ... }:

let
  cfg = config.services.kikoeru-neo;
  toml = pkgs.formats.toml { };
  configFile = toml.generate "kikoeru-neo.toml" cfg.settings;
  # The CLI opens the database serve uses only if it reads the same config,
  # and that file is in the Nix store, not in the data directory where the
  # CLI looks by default. sudo drops the caller's environment, so the
  # defaults go into a wrapper rather than into environment.variables.
  cli = pkgs.symlinkJoin {
    name = "kikoeru-neo-cli";
    paths = [ cfg.package ];
    nativeBuildInputs = [ pkgs.makeWrapper ];
    postBuild = ''
      wrapProgram $out/bin/kikoeru-neo \
        --set-default KIKOERU_CONFIG ${configFile} \
        --set-default KIKOERU_DATA_DIR ${lib.escapeShellArg cfg.dataDir}
    '';
    meta.mainProgram = "kikoeru-neo";
  };
in
{
  options.services.kikoeru-neo = {
    enable = lib.mkEnableOption "kikoeru-neo, a self-hosted player for audio dramas and audiobooks";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "kikoeru-neo.packages.\${system}.default";
      description = "The kikoeru-neo package. Its wrapper puts ffmpeg and ffprobe on PATH.";
    };

    dataDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/kikoeru-neo";
      description = ''
        The database, signing key, covers and caches. Only the service user
        can read it. Back it up with `kikoeru-neo backup`.
      '';
    };

    user = lib.mkOption {
      type = lib.types.str;
      default = "kikoeru-neo";
      description = "The system user the service runs as. It is created when it is the default.";
    };

    group = lib.mkOption {
      type = lib.types.str;
      default = "kikoeru-neo";
      description = "The group of the service user.";
    };

    database.createLocally = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Keep the library in a PostgreSQL database on this machine instead of
        the SQLite file in `dataDir`. This enables services.postgresql and
        creates a database and a role named after `user`, which the service
        reaches over the Unix socket with peer authentication, so no
        password is involved. Move an existing library there first with
        `kikoeru-neo db copy` (docs/en/configuration.md).
      '';
    };

    settings = lib.mkOption {
      inherit (toml) type;
      default = { };
      example = lib.literalExpression ''
        {
          listen = "127.0.0.1:8888";
          public_url = "https://kikoeru.example.lan";
          roots = [
            {
              type = "openlist";
              id = "cloud";
              display_name = "Cloud";
              url = "http://openlist.lan:5244";
              base_path = "/cloud/works";
              username = "kikoeru";
              password_file = "/run/secrets/openlist-password";
              auto_scan_interval = "1d";
            }
            {
              type = "local";
              id = "local";
              display_name = "Local";
              path = "/srv/voice";
              watch = true;
            }
          ];
        }
      '';
      description = ''
        The contents of config.toml (docs/en/configuration.md). It is written to
        the Nix store, so secrets go in files that the settings point at,
        such as `password_file`, never in the settings themselves.

        Each root may set `auto_scan_interval` (off unless set, at least
        "30m" for OpenList or WebDAV and "5m" for local roots) and, for local roots,
        `watch`, which is off on the server unless set.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    users.users = lib.mkIf (cfg.user == "kikoeru-neo") {
      kikoeru-neo = {
        isSystemUser = true;
        inherit (cfg) group;
        home = cfg.dataDir;
      };
    };
    users.groups = lib.mkIf (cfg.group == "kikoeru-neo") { kikoeru-neo = { }; };

    # The server refuses to start on a data directory others can write to,
    # and it is created here rather than with StateDirectory so that it can
    # live outside /var/lib.
    systemd.tmpfiles.settings.kikoeru-neo.${cfg.dataDir}.d = {
      inherit (cfg) user group;
      mode = "0700";
    };

    # For `kikoeru-neo user` and `kikoeru-neo backup`, run as the service
    # user so that new files keep its ownership.
    environment.systemPackages = [ cli ];

    # The socket's file name carries the port, so a server moved off 5432
    # is only found with the port in the URL.
    services.kikoeru-neo.settings.database.url = lib.mkIf cfg.database.createLocally (
      lib.mkDefault (
        "postgres:///${cfg.user}?host=/run/postgresql"
        + "&port=${toString config.services.postgresql.settings.port}&user=${cfg.user}"
      )
    );

    services.postgresql = lib.mkIf cfg.database.createLocally {
      enable = true;
      ensureDatabases = [ cfg.user ];
      ensureUsers = [
        {
          name = cfg.user;
          ensureDBOwnership = true;
        }
      ];
    };

    systemd.services.kikoeru-neo = {
      description = "kikoeru-neo";
      wantedBy = [ "multi-user.target" ];
      wants = [ "network-online.target" ];
      # postgresql.target is reached once the database and role exist.
      requires = lib.optional cfg.database.createLocally "postgresql.target";
      after = [ "network-online.target" ] ++ lib.optional cfg.database.createLocally "postgresql.target";
      environment = {
        KIKOERU_DATA_DIR = cfg.dataDir;
        # Limit allocator fragmentation from short-lived remote transfers.
        # The resource-testing rationale is in book/src/testing.md (Chinese).
        MALLOC_ARENA_MAX = "2";
      };
      # A failed migration exits non-zero. The start limit stops the restarts
      # from looping, so the unit stays failed with the exit code recorded.
      startLimitIntervalSec = 300;
      startLimitBurst = 5;
      serviceConfig = {
        ExecStart = "${lib.getExe cfg.package} serve --config ${configFile}";
        User = cfg.user;
        Group = cfg.group;
        Restart = "on-failure";
        RestartSec = 5;
        UMask = "0077";
        ReadWritePaths = [ cfg.dataDir ];

        NoNewPrivileges = true;
        CapabilityBoundingSet = "";
        AmbientCapabilities = "";
        ProtectSystem = "strict";
        # Local roots may sit under /home, which the service only reads.
        ProtectHome = "read-only";
        PrivateTmp = true;
        PrivateDevices = true;
        ProtectClock = true;
        ProtectHostname = true;
        ProtectKernelLogs = true;
        ProtectKernelModules = true;
        ProtectKernelTunables = true;
        ProtectControlGroups = true;
        ProtectProc = "invisible";
        RestrictAddressFamilies = [ "AF_INET" "AF_INET6" "AF_UNIX" ];
        RestrictNamespaces = true;
        RestrictRealtime = true;
        # RestrictSUIDSGID stays off. systemd cannot inspect openat2's
        # flags, so it refuses openat2 outright, and local roots would lose
        # the kernel's RESOLVE_BENEATH lookup. Without capabilities the
        # service could only mark files setuid to its own user, inside a
        # data directory nobody else can enter.
        LockPersonality = true;
        RemoveIPC = true;
        SystemCallArchitectures = "native";
        SystemCallFilter = [ "@system-service" "~@privileged" ];
      };
    };
  };
}
