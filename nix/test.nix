# Boots the module in a VM: the service starts, an administrator created
# from the command line can log in, a local root's scan settings reach the
# server and the sandbox lets it scan that root, a backup can be taken
# while it runs, and restoring that backup by
# the steps in docs/en/backup-restore.md works. A second machine keeps the library
# in a local PostgreSQL database, and its CLI finds that database through
# the module's wrapper.
# `package` is the unstamped build the other checks share, so a commit that
# does not touch the sources leaves this test cached.
{ self, pkgs, package }:

pkgs.testers.runNixOSTest {
  name = "kikoeru-neo";
  nodes.machine = {
    imports = [ self.nixosModules.default ];
    services.kikoeru-neo = {
      enable = true;
      inherit package;
      settings = {
        listen = "127.0.0.1:8888";
        # The scan settings pass through the module to the server.
        roots = [{
          type = "local";
          id = "local";
          display_name = "Local";
          path = "/srv/voice";
          watch = true;
          auto_scan_interval = "1h";
        }];
      };
    };
    systemd.tmpfiles.rules = [ "d /srv/voice/RJ000001 0755 root root -" ];
    environment.systemPackages = [ pkgs.curl pkgs.jq ];
  };
  nodes.pg = {
    imports = [ self.nixosModules.default ];
    services.kikoeru-neo = {
      enable = true;
      inherit package;
      database.createLocally = true;
      settings.listen = "127.0.0.1:8888";
    };
    # Off the default port, so a URL that forgot the port finds no socket.
    services.postgresql.settings.port = 5433;
    environment.systemPackages = [ pkgs.curl pkgs.jq ];
  };
  testScript = ''
    as_service = "runuser -u kikoeru-neo -- kikoeru-neo --data-dir /var/lib/kikoeru-neo"

    def login(name, password):
        return (
            "curl -sf -X POST -H 'content-type: application/json'"
            f" -d '{{\"name\":\"{name}\",\"password\":\"{password}\"}}'"
            " http://127.0.0.1:8888/api/auth/login"
        )

    machine.wait_for_unit("kikoeru-neo.service")
    machine.wait_until_succeeds("curl -sf http://127.0.0.1:8888/api/ready")
    assert machine.succeed("stat -c %a /var/lib/kikoeru-neo").strip() == "700"
    assert machine.succeed("stat -c %a /var/lib/kikoeru-neo/secrets/jwt.key").strip() == "600"
    # The running server, not just the unit file, must see the arena cap.
    pid = machine.succeed("systemctl show -p MainPID --value kikoeru-neo").strip()
    machine.succeed(f"tr '\\0' '\\n' < /proc/{pid}/environ | grep -qx MALLOC_ARENA_MAX=2")

    machine.succeed(f"echo 'admin password' | {as_service} user create --admin admin --password-stdin")
    token = machine.succeed(
        login("admin", "admin password") + " | jq -er 'select(.user.is_admin) | .token'"
    ).strip()
    machine.fail(login("admin", "wrong"))

    # The local root's scan settings passed through the module: the server
    # watches it and has its first automatic scan scheduled.
    machine.wait_until_succeeds(
        f"curl -sf -H 'authorization: Bearer {token}' http://127.0.0.1:8888/api/admin/roots"
        " | jq -e '.[0] | .watching and .settings.watch"
        " and .settings.auto_scan_interval == 3600 and (.next_scan_at | type) == \"number\"'"
    )

    # The unit's sandbox must leave the local backend able to open the
    # root. RestrictSUIDSGID once refused openat2 and failed every scan.
    task = machine.succeed(
        f"curl -sf -X POST -H 'authorization: Bearer {token}' -H 'content-type: application/json'"
        " -d '{\"root\":\"local\"}' http://127.0.0.1:8888/api/admin/scan | jq -er '.task_ids[0]'"
    ).strip()
    task_url = f"http://127.0.0.1:8888/api/tasks/{task}"
    machine.wait_until_succeeds(
        f"curl -sf -H 'authorization: Bearer {token}' {task_url}"
        " | jq -e '.state | IN(\"done\", \"failed\", \"cancelled\", \"interrupted\")'"
    )
    print(machine.succeed(f"curl -sf -H 'authorization: Bearer {token}' {task_url}"))
    machine.succeed(f"curl -sf -H 'authorization: Bearer {token}' {task_url} | jq -e '.state == \"done\"'")

    machine.succeed("install -d -o kikoeru-neo -m 700 /var/backup")
    machine.succeed(f"{as_service} backup /var/backup/kikoeru-neo")
    machine.succeed("test -s /var/backup/kikoeru-neo/neo.sqlite3")
    machine.succeed("test -s /var/backup/kikoeru-neo/secrets/jwt.key")

    # A restart keeps the signing key, so the account still works.
    machine.systemctl("restart kikoeru-neo.service")
    machine.wait_until_succeeds("curl -sf http://127.0.0.1:8888/api/ready")
    machine.succeed(login("admin", "admin password"))

    # Restore drill. A user made after the backup must be gone afterwards,
    # and a token issued before it must still work, which shows that both
    # the database and the signing key came back.
    machine.succeed(f"echo 'later password' | {as_service} user create later --password-stdin")
    machine.succeed(login("later", "later password"))
    machine.systemctl("stop kikoeru-neo.service")
    machine.succeed("mv /var/lib/kikoeru-neo /var/lib/kikoeru-neo.broken")
    machine.succeed("install -d -o kikoeru-neo -g kikoeru-neo -m 700 /var/lib/kikoeru-neo")
    machine.succeed("cp -a /var/backup/kikoeru-neo/. /var/lib/kikoeru-neo/")
    machine.systemctl("start kikoeru-neo.service")
    machine.wait_until_succeeds("curl -sf http://127.0.0.1:8888/api/ready")
    machine.succeed(
        f"curl -sf -H 'authorization: Bearer {token}' http://127.0.0.1:8888/api/auth/me"
        " | jq -e '.name == \"admin\"'"
    )
    machine.fail(login("later", "later password"))
    assert machine.succeed("stat -c %a /var/lib/kikoeru-neo/secrets/jwt.key").strip() == "600"

    # PostgreSQL. The CLI is given no --data-dir or config, so the user it
    # creates only reaches the server if the wrapper points the CLI at the
    # service's config and through it at the same database.
    pg.wait_for_unit("kikoeru-neo.service")
    pg.wait_until_succeeds("curl -sf http://127.0.0.1:8888/api/ready")
    pg.succeed("echo 'admin password' | runuser -u kikoeru-neo -- kikoeru-neo user create --admin admin --password-stdin")
    pg.succeed(login("admin", "admin password") + " | jq -e '.user.is_admin'")
    psql = "runuser -u postgres -- psql -p 5433 -tAc"
    assert pg.succeed(f"{psql} 'SELECT name FROM users' kikoeru-neo").strip() == "admin"
    pg.fail("test -e /var/lib/kikoeru-neo/neo.sqlite3")

    # A backup exports the PostgreSQL library to a SQLite file.
    pg.succeed("install -d -o kikoeru-neo -m 700 /var/backup")
    pg.succeed("runuser -u kikoeru-neo -- kikoeru-neo backup /var/backup/kikoeru-neo")
    pg.succeed("test -s /var/backup/kikoeru-neo/neo.sqlite3")
    pg.succeed("test -s /var/backup/kikoeru-neo/secrets/jwt.key")

    # Move a SQLite library as described in docs/en/configuration.md.
    # The SQLite library here is made with the CLI, which reads the data
    # directory's own config once KIKOERU_CONFIG is cleared.
    pg.succeed(
        "install -d -o kikoeru-neo -m 700 /var/lib/old"
        " && echo 'old password' | runuser -u kikoeru-neo -- env KIKOERU_CONFIG="
        " kikoeru-neo --data-dir /var/lib/old user create --admin old --password-stdin"
    )
    pg.systemctl("stop kikoeru-neo.service")
    pg.succeed("runuser -u postgres -- dropdb -p 5433 kikoeru-neo")
    pg.systemctl("restart postgresql-setup.service")
    pg.succeed(
        "runuser -u kikoeru-neo -- kikoeru-neo db copy --from /var/lib/old/neo.sqlite3"
        " --to 'postgres:///kikoeru-neo?host=/run/postgresql&port=5433&user=kikoeru-neo'"
    )
    pg.systemctl("start kikoeru-neo.service")
    pg.wait_until_succeeds("curl -sf http://127.0.0.1:8888/api/ready")
    pg.succeed(login("old", "old password"))
    pg.fail(login("admin", "admin password"))
  '';
}
